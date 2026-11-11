//! 開發環境墊片：gpui（x11 feature）連結 `-lxkbcommon-x11`，Debian 需要
//! `libxkbcommon-x11-dev` 才有不帶版號的 `libxkbcommon-x11.so` 連結用 symlink。
//! 若系統只有執行期的 `libxkbcommon-x11.so.0`（常見），在 OUT_DIR 補一個 symlink
//! 讓連結器找得到；正式做法仍是 `sudo apt install libxkbcommon-x11-dev`（見 README）。
//! 不含任何業務邏輯。

use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let lib_dirs = [
        "/usr/lib/x86_64-linux-gnu",
        "/usr/lib64",
        "/usr/lib",
        "/usr/local/lib",
    ];
    let has_dev_link = lib_dirs
        .iter()
        .any(|d| Path::new(d).join("libxkbcommon-x11.so").exists());
    if has_dev_link {
        return;
    }
    let Some(runtime) = lib_dirs
        .iter()
        .map(|d| Path::new(d).join("libxkbcommon-x11.so.0"))
        .find(|p| p.exists())
    else {
        return; // 連執行期函式庫都沒有：讓連結器照常報錯，訊息較清楚
    };
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    let shim_dir = Path::new(&out_dir).join("linkshim");
    std::fs::create_dir_all(&shim_dir).expect("create linkshim dir");
    let link = shim_dir.join("libxkbcommon-x11.so");
    if !link.exists() {
        #[cfg(unix)]
        std::os::unix::fs::symlink(&runtime, &link).expect("create libxkbcommon-x11.so shim");
    }
    println!("cargo:rustc-link-search=native={}", shim_dir.display());
}
