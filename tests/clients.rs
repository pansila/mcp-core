use mcp_core::{
    client::{ClientBuilder, SecureValue},
    error::McpError,
};

#[tokio::test]
async fn test_client_env() -> Result<(), McpError> {
    let sec_client = ClientBuilder::new()
        .with_secure_value("test_key", SecureValue::Static("test_value".to_string()))
        .build();

    let secure_value = sec_client.get_secure_value("test_key").await?;
    assert_eq!(secure_value, "test_value");

    let mut json_value = serde_json::json!({
        "test_key": "",
        "container": {
            "test_key": ""
        },
        "array": [
            {
                "test_key": ""
            }
        ],
        "container_two": {
            "test_key": {}
        },
        "container_three": {
            "test_key": {
                "test_key": ""
            }
        }
    });

    sec_client
        .apply_secure_replacements(&mut json_value)
        .await?;

    // Debug print the key and the value of json_value
    println!("Key: test_key, Value: {}", json_value["test_key"]);

    assert_eq!(json_value["test_key"], secure_value);

    Ok(())
}
