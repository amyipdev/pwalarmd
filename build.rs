fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=src/sock.proto,proto");
    std::fs::create_dir_all("proto")?;
    let mut pcg = protobuf_codegen::Codegen::new();
    pcg.pure();
    pcg.out_dir("proto");
    pcg.include("src");
    pcg.input("src/sock.proto");
    pcg.run_from_script();
    Ok(())
}
