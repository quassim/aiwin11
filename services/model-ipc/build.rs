fn main() -> Result<(), Box<dyn std::error::Error>> {
    let schema = "../../contracts/v1/model_ipc.proto";
    println!("cargo:rerun-if-changed={schema}");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut prost = tonic_prost_build::Config::new();
        prost.protoc_executable(protoc_bin_vendored::protoc_bin_path()?);
        tonic_prost_build::configure().compile_with_config(prost, &[schema], &["../../contracts/v1"])?;
    }
    Ok(())
}
