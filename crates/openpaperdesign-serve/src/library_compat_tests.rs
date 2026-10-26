//! R84-01 (run 195): an older OpenPaperDesign process sharing a library canvas's handover record
//! with this one. The older process is the exact library of runs 192 and 193 (`library_192.rs`,
//! byte for byte `crates/openpaperdesign-serve/src/library.rs` as frozen then: 92310 bytes,
//! SHA-256 `1e1c2595889e3544307ff8980fdf650d8924955126d10452630081cca5fc7ff2`, checked by
//! `crates/openpaperdesign-web/tests/session.test.mjs`; it is the same file in both runs), on its
//! own library handle, lease handle and editing identity, as another process has them. Its
//! server's offer actions (`lib.rs` `apply_offer`, `keep_offer`, `dismiss_offer` of then) are
//! the library calls they make, in their order: the commit or new canvas between finding an
//! offer and removing it is the old server's, and nothing of an offer's continuations goes with
//! either.

use super::*;
use crate::library_192 as old;
use crate::test_support::Point;

/// A library canvas no view has opened yet (made as a create makes it; no handover record).
struct Canvas {
    root: PathBuf,
    id: String,
}

impl Canvas {
    fn new() -> Canvas {
        use std::hash::{BuildHasher, RandomState};
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "opd-compat-{:016x}",
            RandomState::new().hash_one(std::time::SystemTime::now())
        ));
        std::fs::create_dir_all(&root).unwrap();
        let lib = Library::open(root.to_str().unwrap()).unwrap();
        let lease = Lease::fresh(&lib).unwrap();
        lib.create_document(&lease, &NewDocument::blank("Plan").fresh())
            .unwrap();
        let id = lease.doc.clone();
        Canvas { root, id }
    }

    fn locks(&self) -> PathBuf {
        self.root.join("library/v1/locks")
    }

    /// The handover record's bytes (`None`: there is none).
    fn record(&self) -> Option<Vec<u8>> {
        std::fs::read(self.locks().join(format!("{}.handover", self.id))).ok()
    }

    fn record_json(&self) -> Value {
        serde_json::from_slice(&self.record().unwrap()).unwrap()
    }

    /// The offer canvas files beside it.
    fn offer_files(&self) -> usize {
        let prefix = format!("{}.offer-", self.id);
        std::fs::read_dir(self.locks())
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(&prefix)
            })
            .count()
    }

    /// A binding of this OpenPaperDesign whose view `view` opens the canvas (and so takes the
    /// editing): its claim's note.
    fn current(&self, view: &str) -> (Current, Option<String>) {
        let b = self.unclaimed(view);
        let note = b.claim().unwrap();
        (b, note)
    }

    /// A binding of this OpenPaperDesign with view `view` that hasn't taken the editing.
    fn unclaimed(&self, view: &str) -> Current {
        let lib = Arc::new(Library::open(self.root.to_str().unwrap()).unwrap());
        let lease = Arc::new(Lease::acquire(&lib, &self.id).ok().unwrap());
        let ed = Editing::new(lib, &lease).unwrap();
        lease.unlock();
        Current {
            _lease: lease,
            ed,
            broken: crate::Broken::default(),
            view: view.into(),
        }
    }

    /// A binding of the older OpenPaperDesign (another process) whose view `view` opens the
    /// canvas: its claim's note.
    fn old(&self, view: &str) -> (Old, Option<String>) {
        let lib = Arc::new(old::Library::open(self.root.to_str().unwrap()).unwrap());
        let lease = Arc::new(old::Lease::acquire(&lib, &self.id).ok().unwrap());
        let ed = old::Editing::new(lib, &lease).unwrap();
        let broken = crate::Broken::default();
        let note = ed.claim(&broken, Some(view)).ok().unwrap();
        lease.unlock();
        let b = Old {
            lease,
            ed,
            broken,
            view: view.into(),
        };
        (b, note)
    }
}

struct Current {
    _lease: Arc<Lease>,
    ed: Editing,
    broken: crate::Broken,
    view: String,
}

impl Current {
    fn claim(&self) -> Result<Option<String>, String> {
        self.ed
            .claim(&self.broken, Some(&self.view))
            .map_err(|e| e.message())
    }

    /// The editing view resolves offer `id` (its Apply, Keep or Dismiss, saying it holds the
    /// recoveries `resolved`): found under its admission, then removed with their receipts.
    fn resolve(&self, id: &str, resolved: &[String]) -> Result<Offer, String> {
        let hold = self
            .ed
            .admit(&self.broken, Some(&self.view))
            .map_err(|e| e.message())?;
        let (offer, _) = self
            .ed
            .find_offer(&hold, id)
            .map_err(|e| e.message())?
            .ok_or("404")?;
        self.ed
            .remove_offer(&hold, &self.broken, id, resolved)
            .map_err(|e| e.message())?;
        Ok(offer)
    }
}

struct Old {
    lease: Arc<old::Lease>,
    ed: old::Editing,
    broken: crate::Broken,
    view: String,
}

impl Old {
    fn claim(&self) -> Result<Option<String>, String> {
        self.ed
            .claim(&self.broken, Some(&self.view))
            .map_err(|e| e.message())
    }

    /// Its view (which lost the editing) answers the handover with `offer`.
    fn answer(&self, offer: Value) -> Result<Option<String>, String> {
        let o = old::NewOffer {
            base: offer["base"].as_str().unwrap().into(),
            canvas: offer["canvas"].as_str().unwrap().into(),
            changed: offer["changed"].as_bool().unwrap(),
            unfinished: offer["unfinished"].as_array().unwrap().clone(),
        };
        self.ed
            .answer(&self.broken, &self.view, Some(o))
            .map_err(|e| e.message())
    }

    /// What its view is shown of the record (`lib.rs` `handover_json`: null when nothing).
    fn shown(&self) -> Option<Value> {
        self.ed.handover(Some(&self.view))
    }

    /// Its editing view's `action` on offer `id`, as its server makes it (`lib.rs` of runs
    /// 192/193: `apply_offer` 891–916, `keep_offer` 923–952, `dismiss_offer` 957–970; its
    /// Continue here dismisses an offer it took everything it knows from, session.js 688–695).
    fn act(&self, action: &str, id: &str) -> Result<(), String> {
        let hold = self
            .ed
            .admit(&self.broken, Some(&self.view))
            .map_err(|e| e.message())?;
        let (offer, canvas) = self
            .ed
            .find_offer(&hold, id)
            .map_err(|e| e.message())?
            .ok_or("404")?;
        if action == "apply" && !(offer.changed && canvas.is_some()) {
            return Err("404".into());
        }
        if action == "keep" && canvas.is_none() {
            return Err("404".into());
        }
        // (Apply commits the offer's canvas here, Keep makes a new canvas of it.)
        self.ed
            .remove_offer(&hold, &self.broken, id)
            .map_err(|e| e.message())
    }
}

/// A kept recovery (session.js `recoveries`, R82-01): A's Canvas name, applied on `"r1"`.
fn recovery() -> Value {
    json!({"kind": "recovery", "v": 1, "id": "rec-a", "state": "applied", "revision": "\"r1\"",
        "packets": [{"kind": "title", "base": "Canvas", "text": "Dz-195-æøå"}]})
}

/// The Canvas name B typed of its own.
fn name_e() -> Value {
    json!({"kind": "title", "base": "Canvas", "text": "E-195"})
}

/// The mixed answers a run-193 view sends (session.js 462–466 of then): its own change with
/// the recovery, or its unfinished name with it.
fn mixed() -> [(&'static str, Value); 2] {
    [
        (
            "apply",
            json!({"base": "\"r1\"", "canvas": "{\"pages\":[]}", "changed": true,
                "unfinished": [recovery()]}),
        ),
        (
            "dismiss",
            json!({"base": "\"r1\"", "canvas": "", "changed": false,
                "unfinished": [name_e(), recovery()]}),
        ),
    ]
}

/// The step-by-step schedule (R84-01): B (older) had the editing; C (this OpenPaperDesign)
/// opens and takes it; B's older server answers with its mixed offer; D (older) opens and does
/// what its banner offers. Nothing of it may be consumed: B's answer is refused before anything
/// is written, D finds nothing, and the record stays as C wrote it.
#[test]
fn r195_an_older_answer_after_this_version_joined_is_refused_before_anything_is_written() {
    for (action, offer) in mixed() {
        let k = Canvas::new();
        let (b, _) = k.old("t1");
        let (c, note) = k.current("m1");
        assert_eq!(note, None, "{action}");
        let joined = k.record();
        let answered = b.answer(offer.clone());
        let (d, d_note) = k.old("t1");
        let shown = d.shown();
        let acted: Vec<String> = shown
            .as_ref()
            .map(|h| h["offers"].as_array().unwrap().clone())
            .unwrap_or_default()
            .iter()
            .map(|o| format!("{:?}", d.act(action, o["id"].as_str().unwrap())))
            .collect();
        let outcome = json!({
            "answer": answered.as_ref().err().map(|e| e.contains("unreadable")),
            "d_shown": shown,
            "d_acted": acted,
            "d_claim": d_note.as_ref().map(|n| n.contains("couldn't be read")),
            "record_as_c_wrote_it": k.record() == joined,
            "offer_files": k.offer_files(),
        });
        assert_eq!(
            outcome,
            json!({"answer": true, "d_shown": null, "d_acted": [], "d_claim": true,
                "record_as_c_wrote_it": true, "offer_files": 0}),
            "{action}: {answered:?}"
        );
        // C's record waits for B, as written (B never answered).
        let polled = c.ed.handover(Some("m1")).unwrap();
        assert_eq!(polled["offers"], json!([]), "{action}");
        assert_eq!(polled["pending"].as_array().unwrap().len(), 1, "{action}");
        // B's older server again (a later poll's answer, a reconnect's claim), D's close: nothing.
        assert!(b.answer(offer).is_err(), "{action}");
        assert!(b.claim().unwrap().unwrap().contains("couldn't be read"));
        d.ed.release_view(&d.broken, "t1");
        d.ed.release(&d.broken);
        assert_eq!(k.record(), joined, "{action}: still as C wrote it");
        drop(d.lease);
    }
}

/// A mixed offer an older process wrote before this version joined (the record then is the
/// older one's): this version carries it over whole when it writes the record, which older
/// processes then can't read. Their Apply, Keep, Dismiss (Continue here), stop waiting, claim,
/// close and answer are all refused before anything happens, and the newest view of this version
/// still finds the recovery exactly as it was handed over, and resolves it only with its
/// receipt.
#[test]
fn r195_an_older_record_is_carried_over_whole_and_no_older_process_consumes_it_then() {
    for (action, offer) in mixed() {
        let k = Canvas::new();
        let (b, _) = k.old("t1");
        let (_x, _) = k.old("t1");
        let made = b.answer(offer.clone()).unwrap().unwrap();
        let before = k.record_json();
        assert!(before.get("format").is_none(), "an older record");
        let m = before["offers"][0].clone();
        assert_eq!(
            m["unfinished"], offer["unfinished"],
            "{action}: as B sent it"
        );
        // C (this version) opens: the record is written again, M in it.
        let (c, note) = k.current("m1");
        assert_eq!(note, None, "{action}");
        let carried = k.record();
        // D (older) opens and does what its banner offers (and the rest it could); B polls.
        let (d, d_note) = k.old("t1");
        let shown = d.shown();
        let acted: Vec<String> = ["apply", "keep", "dismiss"]
            .iter()
            .map(|a| match d.act(a, &made) {
                Ok(()) => format!("{a}: consumed"),
                Err(e) if e.contains("unreadable") => format!("{a}: refused"),
                Err(e) => format!("{a}: {e}"),
            })
            .collect();
        let outcome = json!({
            "d_claim": d_note.as_ref().map(|n| n.contains("couldn't be read")),
            "d_shown": shown,
            "d_acted": acted,
            "b_shown": b.shown(),
            "b_answer_refused": b.answer(offer.clone()).is_err(),
            "record_as_c_wrote_it": k.record() == carried,
        });
        assert_eq!(
            outcome,
            json!({"d_claim": true, "d_shown": null,
                "d_acted": ["apply: refused", "keep: refused", "dismiss: refused"],
                "b_shown": null, "b_answer_refused": true, "record_as_c_wrote_it": true}),
            "{action}"
        );
        // Written in this version's own form, M carried over whole.
        let now = k.record_json();
        assert_eq!(now["format"], 2, "{action}");
        assert_eq!(now["handover"]["offers"], json!([m]), "{action}: M exactly");
        d.ed.release_view(&d.broken, "t1");
        d.ed.release(&d.broken);
        assert_eq!(k.record(), carried, "{action}: byte for byte");
        assert_eq!(k.offer_files(), usize::from(action == "apply"));
        // The newest view of this version: M, exactly, for it to resolve.
        let (e, _) = k.current("m2");
        let shown = e.ed.handover(Some("m2")).unwrap();
        let listed = shown["offers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|o| o["id"] == made.as_str())
            .unwrap()
            .clone();
        assert_eq!(listed["unfinished"], offer["unfinished"], "{action}");
        assert_eq!(listed["base"], "\"r1\"");
        let resolved = e.resolve(&made, &["rec-a".into()]).unwrap();
        assert_eq!(
            resolved.unfinished,
            offer["unfinished"].as_array().unwrap().clone()
        );
        let after = k.record_json();
        assert_eq!(
            after["handover"]["resolved"],
            json!([{"offer": made, "recovery": "rec-a"}])
        );
        assert_eq!(after["format"], 2);
        let _ = c;
    }
}

/// The record's form lasts once this version wrote it: emptied (nothing pending, no offer)
/// it stays, so an older process's later answer is refused all the same.
#[test]
fn r195_an_emptied_record_keeps_older_processes_out() {
    let k = Canvas::new();
    let (c, _) = k.current("m1");
    assert_eq!(k.record_json()["format"], 2, "written by the first claim");
    let (b, _) = k.old("t1");
    c.claim().unwrap();
    let waiting = c.ed.handover(Some("m1")).unwrap()["pending"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let hold = c.ed.admit(&c.broken, Some("m1")).ok().unwrap();
    assert!(c.ed.stop_waiting(&hold, &c.broken, &waiting).ok().unwrap());
    drop(hold);
    let empty = k.record();
    assert_eq!(
        k.record_json(),
        json!({"format": 2, "handover": {"offers": [], "pending": [], "resolved": []}})
    );
    assert_eq!(c.ed.handover(Some("m1")), None, "nothing for a view");
    for (_, offer) in mixed() {
        assert!(b.answer(offer).is_err());
    }
    b.ed.release(&b.broken);
    assert_eq!(k.record(), empty);
}

/// A record this version doesn't know (a newer form, or its own form unreadable) is never
/// overwritten or read as empty: a claim stands with a note, everything else is refused, the
/// bytes stay.
#[test]
fn r195_an_unknown_record_is_never_overwritten() {
    for unknown in [
        json!({"format": 3, "handover": {"offers": [], "pending": [], "resolved": []}}),
        json!({"format": 2}),
        json!({"format": 2, "handover": {"offers": [{"id": "x"}], "pending": []}}),
        json!({"format": "2", "offers": [], "pending": []}),
    ] {
        let k = Canvas::new();
        let (c, _) = k.current("m1");
        let path = k.locks().join(format!("{}.handover", k.id));
        std::fs::write(&path, unknown.to_string()).unwrap();
        let bytes = k.record();
        let (b, note) = k.current("m2");
        let note = note.unwrap();
        assert!(note.contains("couldn't be read"), "{unknown}");
        // What it leaves, truthfully (R85-01): nothing reaches this view while it stays so.
        assert!(
            note.contains("left as it is")
                && note.contains("nothing can be handed over to this view")
                && !note.contains("still arrives"),
            "{note}"
        );
        let o = NewOffer {
            base: "\"r1\"".into(),
            canvas: String::new(),
            changed: false,
            unfinished: vec![recovery()],
        };
        assert!(c.ed.answer(&c.broken, "m1", Some(o)).is_err(), "{unknown}");
        assert_eq!(b.ed.handover(Some("m2")), None);
        let hold = b.ed.admit(&b.broken, Some("m2")).ok().unwrap();
        assert!(b.ed.find_offer(&hold, "0123456789abcdef").is_err());
        assert!(
            b.ed.stop_waiting(&hold, &b.broken, "0123456789abcdef")
                .is_err()
        );
        drop(hold);
        c.ed.release_view(&c.broken, "m1");
        assert_eq!(k.record(), bytes, "{unknown}: never overwritten");
    }
}

/// The first write of this version's form over an older record is a checked record change: a
/// failure puts the older record back byte for byte (the claim stands and says so); one that
/// can't be put back fences this server (`Broken`) and keeps the lease locked.
#[test]
fn r195_the_carry_over_is_a_checked_record_change() {
    let k = Canvas::new();
    let (b, _) = k.old("t1");
    let (_x, _) = k.old("t1");
    let (_, offer) = mixed()[0].clone();
    b.answer(offer).unwrap();
    let older = k.record();
    // A view of this version that doesn't edit answers that it has nothing: its first write of
    // the record (no editing record is written before it).
    let c = k.unclaimed("m1");
    {
        let _sync = Point::arm(&k.locks(), "record_sync", false);
        let e = c.ed.answer(&c.broken, "m1", None).err().unwrap().message();
        assert!(e.contains("put back as it was"), "{e}");
    }
    assert_eq!(k.record(), older, "the older record, as it was");
    assert!(crate::refuse_if_broken(&c.broken).is_ok());
    // Both fail: fenced, the lease kept, the record named.
    {
        let _sync = Point::arm(&k.locks(), "record_sync", false);
        let _restore = Point::arm(&k.locks(), "record_restore", false);
        let e = c.ed.answer(&c.broken, "m1", None).err().unwrap().message();
        assert!(e.contains("may now be either version"), "{e}");
    }
    assert!(crate::refuse_if_broken(&c.broken).is_err());
    let other = old::Library::open(k.root.to_str().unwrap()).unwrap();
    assert!(matches!(
        old::Lease::acquire(&other, &k.id),
        Err(old::LeaseError::Busy)
    ));
    assert!(c.claim().is_err(), "refused while fenced");
}

/// R85-01 (run 196): a claim whose handover bookkeeping failed after it took the editing is a
/// claim (its note says the handover isn't confirmed, never that it still arrives), and it is
/// not the activation of this version's form: the older record stays as it was, readable to an
/// older process, until a write of this version's form succeeds.
#[test]
fn r196_a_claim_whose_record_write_failed_is_not_the_activation() {
    let k = Canvas::new();
    let (b, _) = k.old("t1");
    let (_x, _) = k.old("t1");
    let (_, offer) = mixed()[0].clone();
    let made = b.answer(offer).unwrap().unwrap();
    let older = k.record();
    let c = k.unclaimed("m1");
    let note = {
        let _write = Point::arm(&k.locks(), "handover_write", false);
        c.claim().unwrap().unwrap()
    };
    assert!(
        note.contains("Editing was taken here")
            && note.contains("isn't confirmed here")
            && !note.contains("still arrives"),
        "{note}"
    );
    assert!(c.ed.owns(Some("m1")).ok().unwrap(), "taken");
    assert_eq!(k.record(), older, "not written: still the older form");
    let (d, d_note) = k.old("t1");
    assert_eq!(d_note, None, "an older process still reads it");
    assert!(d.shown().is_some());
    // This version's next successful write is the activation.
    let (_e, note) = k.current("m2");
    assert_eq!(note, None);
    assert_eq!(k.record_json()["format"], 2);
    let listed = &k.record_json()["handover"]["offers"];
    assert!(
        listed
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["id"] == made.as_str())
    );
    let (d2, note) = k.old("t1");
    assert!(note.unwrap().contains("couldn't be read"));
    assert_eq!(d2.shown(), None);
}
