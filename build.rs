fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "grpc")]
    {
        // 尝试编译 proto 文件
        // 如果 protoc 不可用，会输出警告但不会失败
        match tonic_build::configure()
            .build_server(true)
            .build_client(true)
            .compile_protos(&["proto/bridge.proto"], &["proto"])
        {
            Ok(_) => {}
            Err(e) => {
                // 检查是否是因为 protoc 未找到
                let error_msg = e.to_string();
                if error_msg.contains("protoc") || error_msg.contains("NotFound") {
                    println!("cargo:warning=protoc not found, gRPC code generation skipped");
                    println!("cargo:warning=To enable gRPC support, install protobuf:");
                    println!("cargo:warning=  macOS: brew install protobuf");
                    println!("cargo:warning=  Ubuntu: apt install protobuf-compiler");
                } else {
                    return Err(e.into());
                }
            }
        }
    }
    Ok(())
}
