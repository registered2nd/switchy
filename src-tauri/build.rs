fn main() {
    // Embed a small Windows launcher so manual sign-in needs no installed compiler.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let source = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
            .join("no_browser.rs");
        let output =
            std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("no_browser.exe");
        let status = std::process::Command::new(std::env::var_os("RUSTC").unwrap())
            .arg(&source)
            .args([
                "--edition=2021",
                "--crate-name=switchy_no_browser",
                "--target",
            ])
            .arg(std::env::var("TARGET").unwrap())
            .args(["-C", "opt-level=s", "-C", "panic=abort", "-o"])
            .arg(&output)
            .status()
            .expect("could not compile the manual sign-in browser launcher");
        assert!(
            status.success(),
            "manual sign-in browser launcher compilation failed"
        );
        println!("cargo:rerun-if-changed={}", source.display());
    }
    tauri_build::build();
    write_command_catalog();

    // Windows: Embed Common Controls v6 manifest for test binaries
    //
    // When running `cargo test`, the generated test executables don't include
    // the standard Tauri application manifest. Without Common Controls v6,
    // `tauri::test` calls fail with STATUS_ENTRYPOINT_NOT_FOUND.
    //
    // This workaround:
    // 1. Embeds the manifest into test binaries via /MANIFEST:EMBED
    // 2. Uses /MANIFEST:NO for the main binary to avoid duplicate resources
    //    (Tauri already handles manifest embedding for the app binary)
    #[cfg(target_os = "windows")]
    {
        let manifest_path = std::path::PathBuf::from(
            std::env::var("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR"),
        )
        .join("common-controls.manifest");
        let manifest_arg = format!("/MANIFESTINPUT:{}", manifest_path.display());

        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg={}", manifest_arg);
        // Avoid duplicate manifest resources in binary builds.
        println!("cargo:rustc-link-arg-bins=/MANIFEST:NO");
        println!("cargo:rerun-if-changed={}", manifest_path.display());
    }
}

/// Lists every command the window can call, with the arguments it takes, for
/// `GET /switchy/commands`: the names registered in `generate_handler!` in
/// `src/lib.rs`, each with the parameters of its `#[tauri::command]` function
/// under `src/`, minus the ones Tauri fills in itself (state, app handle,
/// window). Argument names are given in camelCase, as the window sends them.
fn write_command_catalog() {
    let src = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("src");
    println!("cargo:rerun-if-changed={}", src.display());

    let lib = std::fs::read_to_string(src.join("lib.rs")).expect("read src/lib.rs");
    let start = lib.find("generate_handler![").expect("generate_handler! in src/lib.rs");
    let end = start + lib[start..].find(']').expect("end of generate_handler!");
    let names: Vec<String> = lib[start..end]
        .lines()
        .map(|line| line.split("//").next().unwrap_or("").trim())
        .filter_map(|line| line.trim_end_matches(',').rsplit("::").next().map(str::to_string))
        .filter(|name| !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_'))
        .collect();

    let mut signatures = std::collections::HashMap::new();
    let mut files = vec![src.clone()];
    while let Some(path) = files.pop() {
        if path.is_dir() {
            for entry in std::fs::read_dir(&path).unwrap().flatten() {
                files.push(entry.path());
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            for (at, _) in text.match_indices("#[tauri::command") {
                let rest = &text[at..];
                let Some(fn_at) = rest.find("fn ") else { continue };
                let rest = &rest[fn_at + 3..];
                let Some(open) = rest.find('(') else { continue };
                let name = rest[..open].trim().split('<').next().unwrap().to_string();
                signatures.entry(name).or_insert_with(|| command_args(&rest[open + 1..]));
            }
        }
    }

    let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let entries: Vec<String> = names
        .iter()
        .map(|name| {
            let args = match signatures.get(name) {
                Some(args) => format!(
                    "[{}]",
                    args.iter()
                        .map(|(n, t)| format!("{{\"name\":{},\"type\":{}}}", quote(n), quote(t)))
                        .collect::<Vec<_>>()
                        .join(",")
                ),
                None => "null".to_string(),
            };
            format!("{{\"command\":{},\"args\":{}}}", quote(name), args)
        })
        .collect();
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("commands.json");
    std::fs::write(out, format!("[{}]", entries.join(","))).expect("write commands.json");
}

/// The caller-supplied parameters of a command, from the text after its `(`.
fn command_args(params: &str) -> Vec<(String, String)> {
    let params: String = params
        .lines()
        .map(|line| line.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" ");
    let mut depth = 0i32;
    let mut current = String::new();
    let mut parts = Vec::new();
    for c in params.chars() {
        match c {
            '(' | '<' | '[' => depth += 1,
            ')' | '>' | ']' if depth == 0 && c == ')' => break,
            ')' | '>' | ']' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    parts.push(current);
    parts
        .into_iter()
        .filter_map(|part| {
            let mut part = part.trim();
            while part.starts_with("#[") {
                part = part[part.find(']')? + 1..].trim();
            }
            let (name, ty) = part.split_once(':')?;
            let name = name.trim().trim_start_matches("mut ").trim().trim_start_matches('_');
            let ty = ty.split_whitespace().collect::<Vec<_>>().join(" ");
            let filled_by_tauri = ["State<", "AppHandle", "Window", "Webview"]
                .iter()
                .any(|t| ty.contains(t));
            if name.is_empty() || filled_by_tauri {
                return None;
            }
            let mut camel = String::new();
            let mut upper = false;
            for c in name.chars() {
                if c == '_' {
                    upper = true;
                } else if upper {
                    camel.extend(c.to_uppercase());
                    upper = false;
                } else {
                    camel.push(c);
                }
            }
            Some((camel, ty))
        })
        .collect()
}
