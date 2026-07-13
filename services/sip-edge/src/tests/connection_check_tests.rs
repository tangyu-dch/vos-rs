#[tokio::test]
async fn test_database_connection() {
    let config = EdgeConfig::load();
    if let Some(ref db_url) = config.database_url {
        let connect_result = cdr_core::PostgresCdrStore::connect(db_url, config.database_max_connections).await;
        assert!(connect_result.is_ok(), "Database connection test failed: {:?}", connect_result.err());
    } else {
        panic!("Database URL is not configured in config.yaml");
    }
}

#[tokio::test]
async fn test_redis_connection() {
    let config = EdgeConfig::load();
    if let Some(ref redis_url) = config.redis_url {
        let redis_client = redis::Client::open(redis_url.clone());
        assert!(redis_client.is_ok(), "Failed to open Redis client");
        let client = redis_client.unwrap();
        let conn = client.get_multiplexed_tokio_connection().await;
        assert!(conn.is_ok(), "Failed to establish Redis multiplexed connection: {:?}", conn.err());
    } else {
        panic!("Redis URL is not configured in config.yaml");
    }
}

#[tokio::test]
async fn test_nats_connection() {
    let config = EdgeConfig::load();
    if let Some(ref nats_url) = config.nats_url {
        let nats_result = async_nats::connect(nats_url).await;
        assert!(nats_result.is_ok(), "NATS connection test failed: {:?}", nats_result.err());
    }
}

#[tokio::test]
async fn test_s3_storage_connection() {
    let storage_config = storage_core::StorageConfig::from_env();
    let create_result = storage_core::create_storage(&storage_config).await;
    assert!(create_result.is_ok(), "S3/Storage backend creation test failed: {:?}", create_result.err());
}
