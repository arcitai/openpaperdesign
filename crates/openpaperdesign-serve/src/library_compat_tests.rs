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

    /// Another view `view` of this binding takes the editing (it must succeed, with no note).
    fn claim_view(&self, view: &str) {
        assert_eq!(self.ed.claim(&self.broken, Some(view)).ok().unwrap(), None);
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
        // A release can't mark anything in it, and says so (run 199), rather than claiming it did.
        let e = c.ed.release_view(&c.broken, "m1").err().unwrap();
        assert!(
            e.message().contains("unreadable"),
            "{unknown}: {}",
            e.message()
        );
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

/// Run 198: a released view that edits leaves the editing record naming only its binding, a
/// form an older process reads too (its claim then waits for nobody, as this version's does). The
/// record write is a checked one: a failure puts it back (the view stays named, nothing else
/// changes), and one that can't be put back fences this server and keeps the lease. A release of
/// a view of another binding never changes the record.
#[test]
fn r198_a_released_editing_view_leaves_the_record_naming_only_its_binding() {
    let editor = |k: &Canvas| {
        String::from_utf8(std::fs::read(k.locks().join(format!("{}.editor", k.id))).unwrap())
            .unwrap()
    };
    let k = Canvas::new();
    let (c, _) = k.current("m1");
    let named = editor(&k);
    assert!(named.ends_with(":m1"));
    // Another binding's view of the same name leaving changes nothing.
    let other = k.unclaimed("m1");
    assert!(other.ed.release_view(&other.broken, "m1").is_ok());
    assert_eq!(editor(&k), named);
    // A failed write is put back: still named, not fenced, and said (R89-01, run 199).
    let failed = {
        let _sync = Point::arm(&k.locks(), "record_sync", false);
        c.ed.release_view(&c.broken, "m1")
    };
    let e = failed.err().unwrap();
    assert!(matches!(e, EditError::Record(_)), "{}", e.message());
    assert!(
        e.message().contains("put back as it was"),
        "{}",
        e.message()
    );
    assert_eq!(editor(&k), named);
    assert!(crate::refuse_if_broken(&c.broken).is_ok());
    // This binding's next hold of the lease does it first (no second release needed): only the
    // binding.
    drop(c.ed.lock_write(&c.broken).ok().unwrap());
    let token = named.split(':').next().unwrap().to_string();
    assert_eq!(editor(&k), token);
    assert!(c.ed.owns(None).ok().unwrap(), "its agent still edits");
    assert!(!c.ed.owns(Some("m1")).ok().unwrap());
    // The next view to take the editing (another binding's) waits for nobody: the failed
    // release noted that m1 left (run 200, R90-01), an ended entry, the only one.
    let (next, note) = k.current("m2");
    assert_eq!(note, None);
    let pending = next.ed.handover(Some("m2")).unwrap()["pending"].clone();
    assert_eq!(pending.as_array().unwrap().len(), 1, "{pending}");
    assert_eq!(pending[0]["ended"], true, "{pending}");
    let raw = k.record_json()["handover"]["pending"][0]["from"].clone();
    assert_eq!(raw, json!(format!("{token}:m1")));
    // An older process reads the binding-only record as it reads its own, but the handover
    // record (this version's form) it can't: its claim stands and writes nothing there (R84).
    let (old_view, note) = k.old("t1");
    assert!(note.unwrap().contains("couldn't be read"));
    assert_eq!(old_view.shown(), None);
    // Both the write and putting it back fail: fenced, the lease kept.
    let k = Canvas::new();
    let (c, _) = k.current("m1");
    let _sync = Point::arm(&k.locks(), "record_sync", false);
    let _restore = Point::arm(&k.locks(), "record_restore", false);
    let e = c.ed.release_view(&c.broken, "m1").err().unwrap();
    assert!(matches!(e, EditError::Fenced(_)), "{}", e.message());
    assert!(
        e.message().contains("may now be either version") && e.message().contains(".editor"),
        "{}",
        e.message()
    );
    assert!(crate::refuse_if_broken(&c.broken).is_err());
    let other = old::Library::open(k.root.to_str().unwrap()).unwrap();
    assert!(matches!(
        old::Lease::acquire(&other, &k.id),
        Err(old::LeaseError::Busy)
    ));
}

/// R89-01 (run 199): a released view's record cleanup that didn't complete says why, as it
/// happened (the lease busy, the editing record unreadable, the pending entry's write put back),
/// and is done under this binding's next hold of the lease. Offers stay. An editing view's
/// release put back notes that it left (run 200): no other window waits for it.
#[test]
fn r199_a_release_cleanup_that_did_not_complete_says_why_and_is_done_at_the_next_hold() {
    let editor_path = |k: &Canvas| k.locks().join(format!("{}.editor", k.id));
    let pending = |k: &Canvas| -> Vec<(String, bool)> {
        k.record_json()["handover"]["pending"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                (
                    p["from"].as_str().unwrap().to_string(),
                    p["ended"].as_bool().unwrap(),
                )
            })
            .collect()
    };
    // The lease busy (another process holds it for a while): said; done at the next hold.
    let k = Canvas::new();
    let (c, _) = k.current("m1");
    let lib = Library::open(k.root.to_str().unwrap()).unwrap();
    let held = Lease::acquire(&lib, &k.id).ok().unwrap();
    let e = c.ed.release_view(&c.broken, "m1").err().unwrap();
    assert!(matches!(e, EditError::Busy), "{}", e.message());
    assert!(
        std::fs::read_to_string(editor_path(&k))
            .unwrap()
            .ends_with(":m1")
    );
    drop(held);
    drop(c.ed.lock_write(&c.broken).ok().unwrap());
    assert!(
        !std::fs::read_to_string(editor_path(&k))
            .unwrap()
            .contains(':')
    );
    // The editing record unreadable (not a plain file): said; done once it reads again.
    let k = Canvas::new();
    let (c, _) = k.current("m1");
    let named = std::fs::read(editor_path(&k)).unwrap();
    std::fs::remove_file(editor_path(&k)).unwrap();
    std::fs::create_dir(editor_path(&k)).unwrap();
    let e = c.ed.release_view(&c.broken, "m1").err().unwrap();
    assert!(matches!(e, EditError::Failed(_)), "{}", e.message());
    std::fs::remove_dir(editor_path(&k)).unwrap();
    std::fs::write(editor_path(&k), &named).unwrap();
    drop(c.ed.lock_write(&c.broken).ok().unwrap());
    assert!(
        !std::fs::read_to_string(editor_path(&k))
            .unwrap()
            .contains(':')
    );
    // A view waited for (another took the editing) leaves; its pending entry's write is put
    // back: said; its offer stays; the next hold ends the entry.
    let k = Canvas::new();
    let (c, _) = k.current("m1");
    let (_d, _) = k.current("d1");
    let o = NewOffer {
        base: "\"r1\"".into(),
        canvas: String::new(),
        changed: false,
        unfinished: vec![recovery()],
    };
    let (b, _) = k.current("b1");
    let (_e2, _) = k.current("e1");
    let made = b.ed.answer(&b.broken, "b1", Some(o)).ok().unwrap().unwrap();
    let m1 = ":m1";
    assert!(
        pending(&k)
            .iter()
            .any(|(f, ended)| f.ends_with(m1) && !ended)
    );
    let e = {
        let _sync = Point::arm(&k.locks(), "record_sync", false);
        c.ed.release_view(&c.broken, "m1").err().unwrap()
    };
    assert!(matches!(e, EditError::Record(_)), "{}", e.message());
    assert!(
        pending(&k)
            .iter()
            .any(|(f, ended)| f.ends_with(m1) && !ended)
    );
    drop(c.ed.lock_write(&c.broken).ok().unwrap());
    assert!(
        pending(&k)
            .iter()
            .any(|(f, ended)| f.ends_with(m1) && *ended)
    );
    let offers = k.record_json()["handover"]["offers"].clone();
    assert_eq!(offers[0]["id"], made.as_str());
    assert_eq!(offers[0]["unfinished"], json!([recovery()]));
    // The editing view's release put back (run 200, R90-01): the handover record notes that it
    // left, so another window that takes the editing while this one stays idle waits for
    // nobody; this binding's next hold rewrites the editing record (if it still names it).
    let k = Canvas::new();
    let (c, _) = k.current("m1");
    let e = {
        let _sync = Point::arm(&k.locks(), "record_sync", false);
        c.ed.release_view(&c.broken, "m1").err().unwrap()
    };
    assert!(matches!(e, EditError::Record(_)), "{}", e.message());
    assert!(
        e.message().contains("no view waits for it"),
        "{}",
        e.message()
    );
    assert!(
        std::fs::read_to_string(editor_path(&k))
            .unwrap()
            .ends_with(":m1")
    );
    let (x, note) = k.current("x1");
    assert_eq!(note, None, "an ordinary claim");
    assert!(
        pending(&k)
            .iter()
            .all(|(f, ended)| !f.ends_with(m1) || *ended),
        "nobody waits for m1: {:?}",
        pending(&k)
    );
    let shown = x.ed.handover(Some("x1")).unwrap()["pending"].clone();
    assert!(
        shown.as_array().unwrap().iter().all(|p| p["ended"] == true),
        "{shown}"
    );
}

/// R90-01/03 (run 200), R91-02 (run 202): neither record can say that the editing view left
/// (the editing record put back, the note in the handover record too; both as they were):
/// said, both causes; the lease kept (not `Broken`: nothing may be either version), so no other
/// window claims and waits for it; this window's next hold records it and lets the lease go.
#[test]
fn r200_a_departure_that_neither_record_can_note_fences_and_keeps_the_lease() {
    let k = Canvas::new();
    let (c, _) = k.current("m1");
    let editor0 = std::fs::read(k.locks().join(format!("{}.editor", k.id))).unwrap();
    let handover0 = k.record();
    let e = {
        let _sync = Point::arm(&k.locks(), "record_sync", false);
        let _note = Point::arm(&k.locks(), "departure_note", false);
        c.ed.release_view(&c.broken, "m1").err().unwrap()
    };
    assert!(matches!(e, EditError::Record(_)), "{}", e.message());
    let m = e.message();
    assert!(
        m.contains("editing record couldn't be written")
            && m.contains("handover record couldn't be written")
            && m.contains("keeps the canvas locked"),
        "{m}"
    );
    assert!(crate::refuse_if_broken(&c.broken).is_ok(), "not fenced");
    assert_eq!(
        std::fs::read(k.locks().join(format!("{}.editor", k.id))).unwrap(),
        editor0
    );
    assert_eq!(k.record(), handover0);
    // No other window can take the editing (and so wait for it): the lease stays held.
    let lib = Library::open(k.root.to_str().unwrap()).unwrap();
    assert!(matches!(Lease::acquire(&lib, &k.id), Err(LeaseError::Busy)));
    // This window's next hold records it (only the binding named), then lets the lease go.
    drop(c.ed.lock_write(&c.broken).ok().unwrap());
    let token = String::from_utf8(editor0).unwrap();
    let token = token.split(':').next().unwrap().to_string();
    assert_eq!(
        std::fs::read_to_string(k.locks().join(format!("{}.editor", k.id))).unwrap(),
        token
    );
    assert!(
        Lease::acquire(&lib, &k.id).is_ok(),
        "the lease goes as usual"
    );
}

/// R90-03 (run 200): a released view's cleanup that is still owed and fails again refuses what
/// relies on the editing record (here this binding's claim): nothing of it runs, the record
/// and the other owner stay; done once it can be. Several owed: the first failure stops them
/// (none after it is tried, so no later fence hides behind it); a fence stops everything and
/// is what is said.
#[test]
fn r200_an_owed_cleanup_that_fails_again_refuses_what_relies_on_it() {
    let editor = |k: &Canvas| std::fs::read(k.locks().join(format!("{}.editor", k.id))).unwrap();
    let live_m1 = |k: &Canvas| {
        k.record_json()["handover"]["pending"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["from"].as_str().unwrap().ends_with(":m1") && p["ended"] == false)
    };
    // m1 (this binding) was waited for by d1 (another), with an offer from b1 (a third).
    let k = Canvas::new();
    let (c, _) = k.current("m1");
    let (_d, _) = k.current("d1");
    assert!(live_m1(&k));
    let e = {
        let _sync = Point::arm(&k.locks(), "record_sync", false);
        c.ed.release_view(&c.broken, "m1").err().unwrap()
    };
    assert!(matches!(e, EditError::Record(_)), "{}", e.message());
    assert!(live_m1(&k), "put back");
    // This binding's view m2 claims while that cleanup still fails: refused, nothing changed.
    let (owner, handover) = (editor(&k), k.record());
    let e = {
        let _sync = Point::arm(&k.locks(), "record_sync", false);
        c.ed.claim(&c.broken, Some("m2")).err().unwrap()
    };
    assert!(matches!(e, EditError::Record(_)), "{}", e.message());
    assert_eq!(editor(&k), owner, "the other owner stays");
    assert_eq!(k.record(), handover);
    assert!(crate::refuse_if_broken(&c.broken).is_ok());
    // Once it can be: done first, then the claim.
    assert_eq!(c.ed.claim(&c.broken, Some("m2")).ok().unwrap(), None);
    assert!(!live_m1(&k));
    assert!(String::from_utf8(editor(&k)).unwrap().ends_with(":m2"));
    // Two owed, both released while the lease was busy (nothing written then): m7 (waited for,
    // since m2 took the editing from it) first, then m2 (editing).
    let k = Canvas::new();
    let (c, _) = k.current("m7");
    c.claim_view("m2");
    let live_m7 = |k: &Canvas| {
        k.record_json()["handover"]["pending"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["from"].as_str().unwrap().ends_with(":m7") && p["ended"] == false)
    };
    assert!(live_m7(&k));
    let lib = Library::open(k.root.to_str().unwrap()).unwrap();
    let held = Lease::acquire(&lib, &k.id).ok().unwrap();
    for v in ["m7", "m2"] {
        let e = c.ed.release_view(&c.broken, v).err().unwrap();
        assert!(matches!(e, EditError::Busy), "{v}: {}", e.message());
    }
    drop(held);
    let owner = editor(&k);
    assert!(String::from_utf8(owner.clone()).unwrap().ends_with(":m2"));
    // The first one's write fails (put back): that is what is said, and m2 is never tried
    // (its editing record unchanged), nothing fenced.
    let e = {
        let _sync = Point::arm(&k.locks(), "record_sync", false);
        c.ed.lock_write(&c.broken).err().unwrap()
    };
    assert!(matches!(e, EditError::Record(_)), "{}", e.message());
    assert!(e.message().contains("handover"), "{}", e.message());
    assert_eq!(editor(&k), owner, "m2 not tried after m7 failed");
    assert!(live_m7(&k));
    assert!(crate::refuse_if_broken(&c.broken).is_ok());
    // Then the first one's write can't be put back: that fence is what is said, and nothing
    // after it runs (no m2 cleanup, no claim): no ownership change after the fence.
    let handover = k.record();
    let e = {
        let _sync = Point::arm(&k.locks(), "record_sync", false);
        let _restore = Point::arm(&k.locks(), "record_restore", false);
        c.ed.claim(&c.broken, Some("m5")).err().unwrap()
    };
    assert!(matches!(e, EditError::Fenced(_)), "{}", e.message());
    assert!(e.message().contains(".handover"), "{}", e.message());
    assert!(crate::refuse_if_broken(&c.broken).is_err());
    assert_eq!(editor(&k), owner, "no ownership change after the fence");
    assert_eq!(k.record().is_some(), handover.is_some());
    let other = Library::open(k.root.to_str().unwrap()).unwrap();
    assert!(matches!(
        Lease::acquire(&other, &k.id),
        Err(LeaseError::Busy)
    ));
}

/// R90-03 (run 200) and R91-01 (run 202): what a binding's own end (`release`) did is kept and
/// said; a busy lease is not its end (its caller waits for it: `release_waiting`), and while
/// it waits nothing is kept as done; once the lease is free the end records that its views
/// ended. A later fence (the handover record may be either version) is what is kept over an
/// earlier editing record put back, and nothing is written after it.
#[test]
fn r200_a_binding_end_says_what_it_did() {
    let k = Canvas::new();
    let c = std::sync::Arc::new(k.current("m1").0);
    assert!(c.ed.retirement().is_none());
    let lib = Library::open(k.root.to_str().unwrap()).unwrap();
    let held = Lease::acquire(&lib, &k.id).ok().unwrap();
    let e = c.ed.release(&c.broken).err().unwrap();
    assert!(matches!(e, EditError::Busy), "{}", e.message());
    assert!(c.ed.retirement().is_none(), "not its end yet");
    let c2 = c.clone();
    let waiting = std::thread::spawn(move || c2.ed.release_waiting(&c2.broken).is_ok());
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert!(!waiting.is_finished(), "under way while the lease is held");
    drop(held);
    assert!(waiting.join().unwrap());
    assert!(matches!(c.ed.retirement(), Some(Ok(()))));
    let rec = std::fs::read(k.locks().join(format!("{}.editor", k.id))).ok();
    assert!(rec.is_none_or(|r| r.is_empty()), "the record names nobody");
    // Completed at once.
    let k = Canvas::new();
    let (c, _) = k.current("m1");
    assert!(c.ed.release(&c.broken).is_ok());
    assert!(matches!(c.ed.retirement(), Some(Ok(()))));
    // The editing record's write put back, then ending its waits can't be put back: the fence
    // (the handover record named) is what is kept, the lease kept, nothing written after it.
    let k = Canvas::new();
    let (c, _) = k.current("m1");
    let (_d, _) = k.current("d1");
    c.claim_view("m2");
    let editor0 = std::fs::read(k.locks().join(format!("{}.editor", k.id))).unwrap();
    let e = {
        let _sync = Point::arm(&k.locks(), "record_sync", false);
        let _restore = Point::arm(&k.locks(), "record_restore", false);
        c.ed.release(&c.broken).err().unwrap()
    };
    assert!(matches!(e, EditError::Fenced(_)), "{}", e.message());
    assert!(e.message().contains(".editor"), "{}", e.message());
    let kept = c.ed.retirement().unwrap().err().unwrap();
    assert!(matches!(kept, EditError::Fenced(_)));
    assert!(crate::refuse_if_broken(&c.broken).is_err());
    let _ = editor0;
    let other = Library::open(k.root.to_str().unwrap()).unwrap();
    assert!(matches!(
        Lease::acquire(&other, &k.id),
        Err(LeaseError::Busy)
    ));
    // One injected failure (R92-01, run 203): the editing record's write put back, nothing
    // else failing. Its waits are ended and the handover record notes that the view it still
    // names left, so another window that takes the editing waits for none of this binding's
    // views; the lease goes as usual.
    let k = Canvas::new();
    let (c, _) = k.current("m1");
    let (_d, _) = k.current("d1");
    c.claim_view("m2");
    let editor0 = std::fs::read(k.locks().join(format!("{}.editor", k.id))).unwrap();
    let named = String::from_utf8(editor0.clone()).unwrap();
    let token = named.split(':').next().unwrap().to_string();
    let e = {
        let _sync = Point::arm(&k.locks(), "record_sync", false);
        c.ed.release(&c.broken).err().unwrap()
    };
    assert!(matches!(e, EditError::Record(_)), "{}", e.message());
    assert!(
        e.message().contains("no view waits for them"),
        "{}",
        e.message()
    );
    assert!(crate::refuse_if_broken(&c.broken).is_ok(), "not fenced");
    assert_eq!(
        std::fs::read(k.locks().join(format!("{}.editor", k.id))).unwrap(),
        editor0,
        "put back"
    );
    let mine = |k: &Canvas| -> Vec<(String, bool)> {
        k.record_json()["handover"]["pending"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["from"].as_str().unwrap().starts_with(&token))
            .map(|p| {
                (
                    p["from"].as_str().unwrap().to_string(),
                    p["ended"].as_bool().unwrap(),
                )
            })
            .collect()
    };
    assert_eq!(
        mine(&k),
        vec![(format!("{token}:m1"), true), (named.clone(), true)]
    );
    let (x, note) = k.current("x1");
    assert_eq!(note, None);
    assert_eq!(
        mine(&k),
        vec![(format!("{token}:m1"), true), (named.clone(), true)],
        "no claim waits for m2 anew"
    );
    // (d1, the other window's view m2 took the editing from, is still open: rightly waited
    // for. It is the only live entry x1 is told.)
    let shown = x.ed.handover(Some("x1")).unwrap();
    let live = shown["pending"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["ended"] == false)
        .count();
    assert_eq!(live, 1, "{shown}");
    drop(x);
    // And the other order (R90-03): the editing record's write put back first (exactly), then
    // the handover record's write can't be put back. That later uncertainty is what is said
    // and kept (the handover record named, not the earlier put-back); the server is fenced
    // (`Broken`, naming it), the lease kept locked, and nothing is written after it.
    let k = Canvas::new();
    let (c, _) = k.current("m1");
    let (_d, _) = k.current("d1");
    c.claim_view("m2");
    let editor0 = std::fs::read(k.locks().join(format!("{}.editor", k.id))).unwrap();
    let e = {
        let _sync = Point::arm(&k.locks(), "record_sync", false);
        let _handover = Point::arm(&k.locks(), "handover_sync", false);
        let _restore = Point::arm(&k.locks(), "handover_restore", false);
        c.ed.release(&c.broken).err().unwrap()
    };
    assert!(matches!(e, EditError::Fenced(_)), "{}", e.message());
    let said = format!("{}.handover may now be either version", k.id);
    assert!(e.message().contains(&said), "{}", e.message());
    assert!(!e.message().contains(".editor"), "{}", e.message());
    let kept = c.ed.retirement().unwrap().err().unwrap();
    assert!(
        matches!(&kept, EditError::Fenced(m) if m.contains(&said)),
        "{}",
        kept.message()
    );
    let fence = crate::refuse_if_broken(&c.broken).unwrap_err();
    assert!(fence.contains(&format!("{}.handover", k.id)), "{fence}");
    assert_eq!(
        std::fs::read(k.locks().join(format!("{}.editor", k.id))).unwrap(),
        editor0,
        "the editing record was put back, and nothing written after the fence"
    );
    let other = Library::open(k.root.to_str().unwrap()).unwrap();
    assert!(matches!(
        Lease::acquire(&other, &k.id),
        Err(LeaseError::Busy)
    ));
    // Nothing of this binding writes after the fence: a later end says the same.
    let again = c.ed.release(&c.broken).err().unwrap();
    assert!(matches!(again, EditError::Fenced(_)), "{}", again.message());
    assert!(matches!(
        Lease::acquire(&other, &k.id),
        Err(LeaseError::Busy)
    ));
}

/// R93-01 (run 204), a view's own release at the same bound: the handover record already lists
/// the most views it can (16, valid): x1 of another window, which this window's m1 took the
/// editing from, and 15 others; m1 has no entry. m1 is released and the editing record's write
/// is put back (it still names m1), and there is no room to note that m1 left. That is not
/// said to be safe: this window keeps the lease, so x1 can't take the editing back (which would
/// first remove its own entry and then wait for m1), and nothing in either record changes. This
/// window's next hold records it (the editing record names only its binding), and lets the
/// lease go; x1 then takes the editing and waits for nobody of this window, every other entry
/// as it was.
#[test]
fn r204_a_released_view_whose_departure_doesnt_fit_a_full_record_keeps_the_lease() {
    let k = Canvas::new();
    let (x, _) = k.current("x1");
    let (c, _) = k.current("m1");
    let path = k.locks().join(format!("{}.handover", k.id));
    let mut v = k.record_json();
    let pending = v["handover"]["pending"].as_array_mut().unwrap();
    assert_eq!(pending.len(), 1, "{pending:?}");
    assert!(pending[0]["from"].as_str().unwrap().ends_with(":x1"));
    for i in 1..16u64 {
        pending.push(json!({"id": format!("00000000002041{i:02x}"),
            "from": format!("{}:x{i}", format!("{i:x}").repeat(64).get(..64).unwrap()),
            "since": 2000 + i, "ended": i % 2 == 1}));
    }
    std::fs::write(&path, v.to_string()).unwrap();
    let full = k.record();
    let before = k.record_json();
    let editor_path = k.locks().join(format!("{}.editor", k.id));
    let editor0 = std::fs::read_to_string(&editor_path).unwrap();
    let token = editor0.split(':').next().unwrap().to_string();
    assert_eq!(editor0, format!("{token}:m1"));
    let e = {
        let _sync = Point::arm(&k.locks(), "record_sync", false);
        c.ed.release_view(&c.broken, "m1").err().unwrap()
    };
    let other = Library::open(k.root.to_str().unwrap()).unwrap();
    let x_claim = x.claim();
    let mine_live = |h: &Value| {
        h["handover"]["pending"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["ended"] == false && p["from"].as_str().unwrap().starts_with(&token))
            .count()
    };
    let outcome = json!({
        "says_no_view_waits": e.message().contains("no view waits"),
        "kind_record": matches!(e, EditError::Record(_)),
        "lease_held": matches!(Lease::acquire(&other, &k.id), Err(LeaseError::Busy)),
        "x_claim_refused": x_claim.is_err(),
        "m1_waited_for": mine_live(&k.record_json()),
        "record_unchanged": k.record() == full,
    });
    assert_eq!(
        outcome,
        json!({"says_no_view_waits": false, "kind_record": true, "lease_held": true,
            "x_claim_refused": true, "m1_waited_for": 0, "record_unchanged": true}),
        "{}",
        e.message()
    );
    assert!(
        e.message().contains("can't wait for more views")
            && e.message().contains("keeps the canvas locked"),
        "{}",
        e.message()
    );
    assert!(crate::refuse_if_broken(&c.broken).is_ok(), "not fenced");
    assert_eq!(std::fs::read_to_string(&editor_path).unwrap(), editor0);
    // This window's next hold records it: the editing record names only its binding.
    drop(c.ed.lock_write(&c.broken).ok().unwrap());
    assert_eq!(std::fs::read_to_string(&editor_path).unwrap(), token);
    assert!(Lease::acquire(&other, &k.id).is_ok(), "the lease goes");
    // x1 takes the editing back: its own entry goes, nobody of this window is waited for, the
    // 15 others exactly as they were.
    assert_eq!(x.claim().unwrap(), None);
    let mut want = before.clone();
    want["handover"]["pending"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    assert_eq!(k.record_json(), want);
}

/// R94-01 (run 205), a view's own release: x1 of another window is waited for (this window's
/// m1 took the editing from it), and m1 is released while reading the editing record fails once
/// (an I/O error, not NotFound; the record is there, valid and unchanged). That isn't taken as
/// "m1 isn't named": the real cause is said (`Failed`), this window keeps the lease, so x1 can't
/// take the editing back (which would wait for m1), and nothing in either record changes. This
/// window's next hold records it (the editing record names only its binding), and lets the
/// lease go; x1 then takes the editing and waits for nobody of this window.
#[test]
fn r205_a_released_view_whose_editing_record_cant_be_read_keeps_the_lease() {
    let k = Canvas::new();
    let (x, _) = k.current("x1");
    let (c, _) = k.current("m1");
    let editor_path = k.locks().join(format!("{}.editor", k.id));
    let editor0 = std::fs::read_to_string(&editor_path).unwrap();
    let token = editor0.split(':').next().unwrap().to_string();
    assert_eq!(editor0, format!("{token}:m1"));
    let before = k.record_json();
    let handover0 = k.record();
    let e = {
        let mut read = Point::arm(&k.locks(), "editor_read", false);
        let e = c.ed.release_view(&c.broken, "m1").err().unwrap();
        reached_now(&mut read);
        e
    };
    let other = Library::open(k.root.to_str().unwrap()).unwrap();
    let x_claim = x.claim();
    let mine_live = |h: &Value| {
        h["handover"]["pending"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["ended"] == false && p["from"].as_str().unwrap().starts_with(&token))
            .count()
    };
    let outcome = json!({
        "kind_failed": matches!(e, EditError::Failed(_)),
        "says_the_read_failed": e.message().contains("injected read failure"),
        "lease_held": matches!(Lease::acquire(&other, &k.id), Err(LeaseError::Busy)),
        "x_claim_refused": x_claim.is_err(),
        "m1_waited_for": mine_live(&k.record_json()),
        "record_unchanged": k.record() == handover0,
        "editor_unchanged": std::fs::read_to_string(&editor_path).unwrap() == editor0,
    });
    assert_eq!(
        outcome,
        json!({"kind_failed": true, "says_the_read_failed": true, "lease_held": true,
            "x_claim_refused": true, "m1_waited_for": 0, "record_unchanged": true,
            "editor_unchanged": true}),
        "{}",
        e.message()
    );
    assert!(
        e.message().contains("keeps the canvas locked"),
        "{}",
        e.message()
    );
    assert!(crate::refuse_if_broken(&c.broken).is_ok(), "not fenced");
    // This window's next hold records it: the editing record names only its binding.
    drop(c.ed.lock_write(&c.broken).ok().unwrap());
    assert_eq!(std::fs::read_to_string(&editor_path).unwrap(), token);
    assert!(Lease::acquire(&other, &k.id).is_ok(), "the lease goes");
    // x1 takes the editing back: its own entry goes, nobody of this window is waited for.
    assert_eq!(x.claim().unwrap(), None);
    let mut want = before.clone();
    want["handover"]["pending"]
        .as_array_mut()
        .unwrap()
        .retain(|p| !p["from"].as_str().unwrap().ends_with(":x1"));
    assert_eq!(k.record_json(), want);
    assert_eq!(mine_live(&k.record_json()), 0);
}

/// LEAD205-01 (run 206), a view's own release: this window's m1 is waited for (x1 of another
/// window took the editing from it), and m1 is released while reading the handover record fails
/// once (an I/O error, not NotFound; the record is there, valid and unchanged). That isn't
/// "no claim can wait on it": m1's entry couldn't be ended, so the real cause is said
/// (`Failed`), this window keeps the lease, and nothing in either record changes. This window's
/// next hold ends m1's entry (only that one) and lets the lease go; x1 is then told of no live
/// wait for m1. A record whose form this version doesn't read stays what it was: no claim can
/// wait on it, so that release is done (the existing unknown-record tests).
#[test]
fn r206_a_released_view_whose_handover_record_cant_be_read_keeps_the_lease() {
    let k = Canvas::new();
    let (c, _) = k.current("m1");
    let (x, _) = k.current("x1");
    let editor_path = k.locks().join(format!("{}.editor", k.id));
    let editor0 = std::fs::read_to_string(&editor_path).unwrap();
    assert!(editor0.ends_with(":x1"), "{editor0}");
    let before = k.record_json();
    let handover0 = k.record();
    let m1_live = |h: &Value| {
        h["handover"]["pending"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|p| p["ended"] == false && p["from"].as_str().unwrap().ends_with(":m1"))
            .count()
    };
    assert_eq!(m1_live(&before), 1, "x1 waits for m1");
    let e = {
        let mut read = Point::arm(&k.locks(), "handover_read", false);
        let e = c.ed.release_view(&c.broken, "m1").err().unwrap();
        reached_now(&mut read);
        e
    };
    let other = Library::open(k.root.to_str().unwrap()).unwrap();
    let outcome = json!({
        "kind_failed": matches!(e, EditError::Failed(_)),
        "says_the_read_failed": e.message().contains("injected read failure"),
        "says_no_view_can_wait": e.message().contains("no view can wait"),
        "lease_held": matches!(Lease::acquire(&other, &k.id), Err(LeaseError::Busy)),
        "record_unchanged": k.record() == handover0,
        "editor_unchanged": std::fs::read_to_string(&editor_path).unwrap() == editor0,
    });
    assert_eq!(
        outcome,
        json!({"kind_failed": true, "says_the_read_failed": true,
            "says_no_view_can_wait": false, "lease_held": true, "record_unchanged": true,
            "editor_unchanged": true}),
        "{}",
        e.message()
    );
    assert!(
        e.message().contains("keeps the canvas locked"),
        "{}",
        e.message()
    );
    assert_eq!(x.ed.handover(Some("x1")).unwrap()["held"], true);
    assert!(crate::refuse_if_broken(&c.broken).is_ok(), "not fenced");
    // This window's next hold records it: m1's entry ended, nothing else changed.
    drop(c.ed.lock_write(&c.broken).ok().unwrap());
    assert!(Lease::acquire(&other, &k.id).is_ok(), "the lease goes");
    let mut want = before.clone();
    for p in want["handover"]["pending"].as_array_mut().unwrap() {
        if p["from"].as_str().unwrap().ends_with(":m1") {
            p["ended"] = json!(true);
        }
    }
    assert_eq!(k.record_json(), want);
    assert_eq!(std::fs::read_to_string(&editor_path).unwrap(), editor0);
    assert_eq!(m1_live(&k.record_json()), 0);
}

/// That a one-shot fault point was reached, from a synchronous test (it was, if it fired).
fn reached_now(p: &mut Point) {
    tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap()
        .block_on(p.reached());
}

/// Run 201 (the R90-01 busy case), the editing view: another process holds the lease past the
/// ordinary wait when it is released. The waiting release stays under way (its thread blocked
/// on the lease itself, nothing written meanwhile), and once that process lets the lease go it
/// leaves the record naming only this binding; another window's claim then waits for nobody.
#[test]
fn r201_an_editing_view_released_while_another_holds_the_lease_is_recorded_once_it_is_free() {
    let k = Canvas::new();
    let c = std::sync::Arc::new(k.current("m1").0);
    let editor = |k: &Canvas| {
        String::from_utf8(std::fs::read(k.locks().join(format!("{}.editor", k.id))).unwrap())
            .unwrap()
    };
    let named = editor(&k);
    let (records0, handover0) = (named.clone(), k.record());
    let lib = Library::open(k.root.to_str().unwrap()).unwrap();
    let held = Lease::acquire(&lib, &k.id).ok().unwrap();
    // The ordinary release meets the busy lease: nothing written.
    let e = c.ed.release_view(&c.broken, "m1").err().unwrap();
    assert!(matches!(e, EditError::Busy), "{}", e.message());
    assert_eq!(editor(&k), records0);
    assert_eq!(k.record(), handover0);
    // The waiting one stays under way while the other holds the lease.
    let c2 = c.clone();
    let waiting = std::thread::spawn(move || c2.ed.release_view_waiting(&c2.broken).ok());
    std::thread::sleep(std::time::Duration::from_millis(1500));
    assert!(!waiting.is_finished(), "under way while the lease is held");
    assert_eq!(editor(&k), records0, "nothing written meanwhile");
    drop(held);
    assert_eq!(waiting.join().unwrap(), Some(()));
    let token = named.split(':').next().unwrap().to_string();
    assert_eq!(editor(&k), token, "only the binding");
    // Another window takes the editing: waits for nobody.
    let (x, note) = k.current("x1");
    assert_eq!(note, None);
    let shown = x.ed.handover(Some("x1"));
    assert!(
        shown.as_ref().is_none_or(|h| h["pending"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["ended"] == true)),
        "{shown:?}"
    );
}
