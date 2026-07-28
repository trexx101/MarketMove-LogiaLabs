use std::env;
use std::process::Command;

fn main() {
    // Only run in debug builds and when not in `cargo test` (which sets CARGO_CFG_TEST).
    // The frontend dist/ must exist for rust-embed to compile.
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let frontend_dir = format!("{}/../frontend", manifest_dir);
    let dist_dir = format!("{}/dist", frontend_dir);

    if !std::path::Path::new(&dist_dir).exists() {
        // Try to build the frontend automatically.
        println!("cargo:warning=frontend/dist/ not found — running npm run build");

        let npm = which_npm();
        if let Some(npm) = npm {
            let result = Command::new(&npm)
                .arg("run")
                .arg("build")
                .current_dir(&frontend_dir)
                .output();

            match result {
                Ok(output) => {
                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        let stdout = String::from_utf8_lossy(&output.stdout);
                        println!("cargo:warning=npm run build failed:\nstdout: {stdout}\nstderr: {stderr}");
                        println!("cargo:warning=Run `cd frontend && npm install && npm run build` manually");
                    }
                }
                Err(e) => {
                    println!("cargo:warning=Failed to run npm: {e}");
                    println!("cargo:warning=Run `cd frontend && npm install && npm run build` manually");
                }
            }
        } else {
            println!("cargo:warning=npm not found — run `cd frontend && npm install && npm run build` manually");
        }
    }

    // Tell cargo to re-run if the frontend dist changes.
    println!("cargo:rerun-if-changed={}/dist", frontend_dir);
}

fn which_npm() -> Option<String> {
    for candidate in ["npm", "npm.cmd"] {
        if Command::new(candidate)
            .arg("--version")
            .output()
            .is_ok()
        {
            return Some(candidate.to_string());
        }
    }
    None
}
