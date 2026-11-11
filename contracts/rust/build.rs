fn main() -> Result<(), Box<dyn std::error::Error>> {
    let schema = "../v1/aiwin11.proto";
    println!("cargo:rerun-if-changed={schema}");
    prost_build::Config::new()
        .protoc_executable(protoc_bin_vendored::protoc_bin_path()?)
        .compile_protos(&[schema], &["../v1"])?;
    Ok(())
}
