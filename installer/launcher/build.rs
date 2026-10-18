// 构建期把 payload 打成 deflate zip 写入 OUT_DIR，由 main.rs 的
// include_bytes!(concat!(env!("OUT_DIR"), "/payload.zip")) 内嵌。
// 依赖的产物（主 exe、卸载器）必须已构建（见 README 构建顺序）。

use std::env;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("launcher crate 应位于 <root>/installer/launcher")
        .to_path_buf();

    let entries: [(String, PathBuf); 5] = [
        (
            "dadaview.exe".to_string(),
            root.join("target/release/dadaview.exe"),
        ),
        ("README.md".to_string(), root.join("README.md")),
        ("LICENSE".to_string(), root.join("LICENSE")),
        ("LICENSE.zh-CN.md".to_string(), root.join("LICENSE.zh-CN.md")),
        (
            "dadaview-uninstall.exe".to_string(),
            root.join("installer/uninstaller/target/release/dadaview-uninstall.exe"),
        ),
    ];

    for (_name, path) in &entries {
        println!("cargo:rerun-if-changed={}", path.display());
        assert!(
            path.exists(),
            "payload 缺失：{}（请按 README 构建顺序先构建主程序与卸载器）",
            path.display()
        );
    }

    // 版本号只维护一处：读根 Cargo.toml 的 [package] version，
    // 注入 DADAVIEW_VERSION，安装器 / 「应用和功能」条目随之自动同步。
    let root_manifest = root.join("Cargo.toml");
    println!("cargo:rerun-if-changed={}", root_manifest.display());
    let manifest = std::fs::read_to_string(&root_manifest).expect("无法读取根 Cargo.toml");
    let version = manifest
        .lines()
        .find_map(|line| {
            let line = line.trim();
            line.strip_prefix("version")?
                .trim()
                .strip_prefix('=')?
                .trim()
                .trim_matches('"')
                .to_string()
                .into()
        })
        .unwrap_or_else(|| "0.0.0".to_string());
    println!("cargo:rustc-env=DADAVIEW_VERSION={}", version);
    println!("cargo:warning=DaDaView 版本号：{}", version);

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let out_path = out_dir.join("payload.zip");
    let file = File::create(&out_path).expect("无法创建 payload.zip");
    let mut zip = ZipWriter::new(file);

    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, path) in &entries {
        let data = std::fs::read(path).expect("无法读取 payload 文件");
        zip.start_file(name.clone(), options)
            .expect("无法写入 zip 条目");
        zip.write_all(&data).expect("无法写入 zip 数据");
    }
    zip.finish().expect("无法完成 payload.zip");
    let size = std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
    println!(
        "cargo:warning=payload.zip 已生成（{} 字节，deflate）",
        size
    );
}
