use std::process::Command;

#[test]
fn preview_writes_a_deterministic_one_bit_frame_and_reports_its_pixel_hash() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("frame.png");
    let preview = || {
        let result = Command::new(env!("CARGO_BIN_EXE_ai-quota-zectrix-dashboard"))
            .args(["preview", "--input", "fixtures/normal.json", "--output"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        (
            std::fs::read(&path).unwrap(),
            String::from_utf8(result.stdout).unwrap(),
        )
    };
    let (png, hash) = preview();
    assert_eq!(&png[16..26], &[0, 0, 1, 144, 0, 0, 1, 44, 1, 0]); // PNG IHDR
    let pixels = image::load_from_memory(&png).unwrap().to_luma8();
    let golden = image::load_from_memory(include_bytes!("../fixtures/normal.png"))
        .unwrap()
        .to_luma8();
    assert_eq!(pixels, golden);
    use sha2::{Digest, Sha256};
    assert_eq!(
        hash.trim(),
        format!("{:x}", Sha256::digest(pixels.as_raw()))
    );
    assert_eq!((png, hash), preview());
}
