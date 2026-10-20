//! Host tests against a real full OTA. Zip path: $EVOX_ZIP or the
//! default EvolutionX shiba build. Manifest-only test is fast (reads
//! kilobytes); the extract test pulls one small partition.

use payload_hs::{extract, manifest, zipx};

fn evox_zip() -> String {
    std::env::var("EVOX_ZIP").unwrap_or_else(|_| {
        "/home/leegarboot/prjects/evox-cnb/out/target/product/shiba/EvolutionX-17.0-20261015-shiba-12.3-Unofficial.zip"
            .to_string()
    })
}

#[test]
fn probe_evox_is_full_ota() {
    let zip = evox_zip();
    let layout = zipx::probe(&zip).expect("zip probe");
    assert!(layout.manifest_len > 1000, "manifest suspiciously small");
    let parts = manifest::parse(&layout.manifest).expect("manifest parse");
    assert!(parts.len() >= 8, "expected 8+ partitions, got {}", parts.len());
    let names: Vec<&str> = parts.iter().map(|p| p.name.as_str()).collect();
    for want in ["system", "vendor", "product", "vendor_boot", "boot"] {
        assert!(names.contains(&want), "missing partition {want} in {names:?}");
    }
    manifest::verdict_full(&parts).expect("must be full OTA");
    // every data op must be decodable offline (no delta ops at all)
    for p in &parts {
        for o in &p.ops {
            assert!(
                manifest::DECODE_ALLOW.contains(&o.typ) || o.typ == manifest::OP_REPLACE_BZ,
                "{}: unexpected op {} ({})",
                p.name,
                o.typ,
                manifest::op_name(o.typ)
            );
        }
    }
}

#[test]
fn extract_vendor_dlkm_parallel() {
    // vendor_dlkm is pure REPLACE_XZ (no bz2 ops) -> fully decodable
    // offline; exercises the parallel path on a real 25 MB partition.
    let zip = evox_zip();
    let layout = zipx::probe(&zip).expect("zip probe");
    let parts = manifest::parse(&layout.manifest).expect("manifest parse");
    let p = parts
        .iter()
        .find(|p| p.name == "vendor_dlkm")
        .expect("vendor_dlkm in manifest");
    let dir = std::env::temp_dir().join("payload_hs_test");
    std::fs::create_dir_all(&dir).unwrap();
    let dst = dir.join("vendor_dlkm.img");
    let st = extract::extract_partition(
        &zip,
        layout.data_start,
        layout.block_size,
        p,
        dst.to_str().unwrap(),
    )
    .expect("extract vendor_dlkm");
    assert!(st.output_bytes > 1_000_000, "output too small: {}", st.output_bytes);
    if p.new_hash.len() == 32 {
        let sum = extract::sha256_file(dst.to_str().unwrap()).expect("sha256");
        assert_eq!(sum[..], p.new_hash[..], "vendor_dlkm sha256 mismatch");
    } else if p.new_size > 0 {
        assert_eq!(st.output_bytes, p.new_size, "size vs new_partition_info");
    }
}
