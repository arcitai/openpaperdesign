//! The canvas shader in upstream `src/render/gpu.rs`, checked against what browser WebGPU
//! requires. The shader text is taken from the upstream source file itself.
//!
//! Browser WebGPU (Tint/Dawn) rejects `textureSample` (implicit derivatives) in non-uniform
//! control flow; native wgpu/naga accepted the old shader, and naga 30's uniformity analysis
//! does not flag that case either. So the uniformity rule is checked structurally here: every
//! implicit-derivative sample in `fs` must come before its first branch or `discard`. This is
//! a focused source check, not a WGSL uniformity analysis.

const GPU_RS: &str = include_str!("../../../src/render/gpu.rs");

fn shader() -> &'static str {
    let start = GPU_RS
        .find("const SHADER: &str = r#\"")
        .expect("SHADER constant in gpu.rs")
        + "const SHADER: &str = r#\"".len();
    let len = GPU_RS[start..].find("\"#;").expect("end of SHADER");
    &GPU_RS[start..start + len]
}

/// Source of `fn <name>` up to the next top-level item (comments stripped).
fn function(name: &str) -> String {
    let s = shader();
    let start = s.find(&format!("fn {name}(")).expect("function in shader");
    let end = s[start..]
        .find("\n}\n")
        .map(|e| start + e + 2)
        .expect("end of function");
    s[start..end]
        .lines()
        .map(|l| l.split("//").next().unwrap())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn canvas_shader_is_valid_wgsl() {
    let module = naga::front::wgsl::parse_str(shader()).expect("WGSL parses");
    let mut v = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    );
    if let Err(e) = v.validate(&module) {
        panic!("{}", e.emit_to_string(shader()));
    }
}

#[test]
fn implicit_derivative_samples_come_before_any_branch() {
    let fs = function("fs");
    let first_branch = ["if (", "if(", "switch", "loop", "for (", "while", "discard"]
        .iter()
        .filter_map(|k| fs.find(k))
        .min()
        .unwrap_or(fs.len());
    let mut samples = 0;
    let mut at = 0;
    while let Some(i) = fs[at..].find("textureSample(") {
        let pos = at + i;
        assert!(
            pos < first_branch,
            "textureSample at byte {pos} of fs is inside or after a branch (first at {first_branch})"
        );
        samples += 1;
        at = pos + 1;
    }
    assert!(samples > 0, "fs samples the image texture");
}
