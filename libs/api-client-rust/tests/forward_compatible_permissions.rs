use daytona_api_client::models::{
    api_key_list::{ApiKeyList, Permissions as ApiKeyListPermission},
    api_key_response::Permissions as ApiKeyResponsePermission,
    create_api_key::Permissions as CreateApiKeyPermission,
    organization_role::Permissions as OrganizationRolePermission,
};

#[test]
fn api_key_list_accepts_mixed_known_and_new_permissions() {
    let api_key: ApiKeyList = serde_json::from_value(serde_json::json!({
        "name": "fabro",
        "value": "dtn_masked",
        "createdAt": "2026-07-31T00:00:00Z",
        "permissions": [
            "write:sandboxes",
            "delete:sandboxes",
            "manage:secrets",
            "read:limits",
            "future:permission"
        ],
        "lastUsedAt": null,
        "expiresAt": null,
        "userId": "user-1"
    }))
    .unwrap();

    assert_eq!(
        api_key.permissions,
        vec![
            ApiKeyListPermission::WRITE_SANDBOXES,
            ApiKeyListPermission::DELETE_SANDBOXES,
            ApiKeyListPermission::MANAGE_SECRETS,
            ApiKeyListPermission::READ_LIMITS,
            ApiKeyListPermission::UnknownDefaultOpenApi,
        ]
    );
}

#[test]
fn response_permissions_accept_unknown_values() {
    let unknown = r#""future:permission""#;

    assert_eq!(
        serde_json::from_str::<ApiKeyListPermission>(unknown).unwrap(),
        ApiKeyListPermission::UnknownDefaultOpenApi
    );
    assert_eq!(
        serde_json::from_str::<ApiKeyResponsePermission>(unknown).unwrap(),
        ApiKeyResponsePermission::UnknownDefaultOpenApi
    );
    assert_eq!(
        serde_json::from_str::<OrganizationRolePermission>(unknown).unwrap(),
        OrganizationRolePermission::UnknownDefaultOpenApi
    );
}

#[test]
fn response_permissions_preserve_known_values() {
    assert_eq!(
        serde_json::from_str::<ApiKeyListPermission>(r#""write:sandboxes""#).unwrap(),
        ApiKeyListPermission::WRITE_SANDBOXES
    );
}

#[test]
fn request_permissions_remain_strict() {
    assert!(serde_json::from_str::<CreateApiKeyPermission>(r#""future:permission""#).is_err());
}
