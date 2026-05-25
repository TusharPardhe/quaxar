use protocol::{
    IsSigning, RuntimeSFieldError, SField, SOEStyle, SOElement, SOTemplate, SerializedTypeId,
    field_code, get_field, get_field_by_name, get_field_by_symbol, max_sfield_num,
    register_runtime_sfield,
};

#[test]
fn protocol_runtime_sfield_registration_uses_shared_lookup_tables_registry_singletons() {
    let field = register_runtime_sfield(
        "sfRuntimeParityRegistryA",
        SerializedTypeId::UInt32,
        9001,
        "RuntimeParityRegistryA",
        SField::S_MD_DEFAULT,
        IsSigning::Yes,
        None,
    )
    .expect("runtime SField should register");

    assert_eq!(field.code(), field_code(SerializedTypeId::UInt32, 9001));
    assert_eq!(get_field(field.code()), field);
    assert_eq!(get_field_by_name("RuntimeParityRegistryA"), field);
    assert_eq!(get_field_by_symbol("sfRuntimeParityRegistryA"), field);
    assert!(field.is_discardable());
    assert!(!field.is_binary());
    assert!(max_sfield_num() >= field.field_num());
}

#[test]
fn protocol_runtime_sfield_registration_rejects_duplicate_code_name_and_symbol() {
    let original = register_runtime_sfield(
        "sfRuntimeParityRegistryB",
        SerializedTypeId::UInt16,
        9010,
        "RuntimeParityRegistryB",
        SField::S_MD_DEFAULT,
        IsSigning::Yes,
        None,
    )
    .expect("original runtime field should register");

    assert_eq!(
        register_runtime_sfield(
            "sfRuntimeParityRegistryBCode",
            SerializedTypeId::UInt16,
            9010,
            "RuntimeParityRegistryBCode",
            SField::S_MD_DEFAULT,
            IsSigning::Yes,
            None,
        ),
        Err(RuntimeSFieldError::DuplicateCode {
            code: field_code(SerializedTypeId::UInt16, 9010),
            existing_symbol: original.symbol_name(),
        })
    );
    assert_eq!(
        register_runtime_sfield(
            "sfRuntimeParityRegistryBName",
            SerializedTypeId::UInt16,
            9011,
            "RuntimeParityRegistryB",
            SField::S_MD_DEFAULT,
            IsSigning::Yes,
            None,
        ),
        Err(RuntimeSFieldError::DuplicateName {
            name: "RuntimeParityRegistryB".to_string(),
            existing_symbol: original.symbol_name(),
        })
    );
    assert_eq!(
        register_runtime_sfield(
            "sfRuntimeParityRegistryB",
            SerializedTypeId::UInt16,
            9012,
            "RuntimeParityRegistryBSymbol",
            SField::S_MD_DEFAULT,
            IsSigning::Yes,
            None,
        ),
        Err(RuntimeSFieldError::DuplicateSymbol {
            symbol_name: "sfRuntimeParityRegistryB".to_string(),
            existing_name: original.name(),
        })
    );
}

#[test]
fn protocol_sotemplate_indexes_runtime_registered_fields_by_field_num() {
    let runtime_field = register_runtime_sfield(
        "sfRuntimeParityRegistryTemplate",
        SerializedTypeId::UInt64,
        9020,
        "RuntimeParityRegistryTemplate",
        SField::S_MD_DEFAULT,
        IsSigning::Yes,
        None,
    )
    .expect("runtime template field should register");
    let runtime_element =
        SOElement::new(runtime_field, SOEStyle::Optional).expect("runtime field should be useful");
    let account_element = SOElement::new(get_field_by_symbol("sfAccount"), SOEStyle::Required)
        .expect("account field should be useful");
    let template = SOTemplate::new(vec![runtime_element], vec![account_element])
        .expect("template should build");

    assert_eq!(
        template
            .get_index(runtime_field)
            .expect("runtime field index should resolve"),
        0
    );
    assert_eq!(
        template
            .get_index(get_field_by_symbol("sfAccount"))
            .expect("account field index should resolve"),
        1
    );
}
