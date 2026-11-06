use doriac::byte_intrinsics::{member, members, type_from_source_name, ByteMemberKind};
use doriac::symbols::{BindingOwnership, ReceiverMode};
use doriac::types::{ByteRegionKind, IntegerType, ResolvedType, TypeRegistry};

#[test]
fn owned_bytes_factories_preserve_argument_binding_contracts() {
    let allocate = member(&ResolvedType::Bytes, "allocate").unwrap();
    let from_array = member(&ResolvedType::Bytes, "fromArray").unwrap();
    assert!(allocate.accepts_named_arguments);
    assert!(!from_array.accepts_named_arguments);
    for factory in [&allocate, &from_array] {
        assert_eq!(factory.kind, ByteMemberKind::StaticMethod);
        assert_eq!(factory.receiver_mode, None);
        assert_eq!(factory.return_type, ResolvedType::Bytes);
        assert_eq!(factory.result_ownership(), BindingOwnership::Owned);
    }
    let mut types = TypeRegistry::new();
    let parameters = allocate.parameter_info(&mut types);
    assert_eq!(parameters[0].name, "length");
    assert_eq!(
        types.resolved(parameters[0].ty),
        ResolvedType::Integer(IntegerType::Int64)
    );
    assert!(!parameters[0].take && !parameters[0].writable && !parameters[0].has_default);
    assert_eq!(
        from_array.parameters[0].ty,
        ResolvedType::TypedArray(Box::new(ResolvedType::Integer(IntegerType::UInt8)))
    );
}

#[test]
fn byte_members_keep_region_access_and_storage_ownership_distinct() {
    for receiver in [
        ResolvedType::Bytes,
        ResolvedType::ByteRegion(ByteRegionKind::Readonly),
        ResolvedType::ByteRegion(ByteRegionKind::Writable),
    ] {
        let writable = receiver != ResolvedType::ByteRegion(ByteRegionKind::Readonly);
        let readonly = member(&receiver, "getRegion").unwrap();
        assert_eq!(
            readonly.result_ownership(),
            BindingOwnership::ReadonlyBorrow
        );
        assert_eq!(readonly.receiver_mode, Some(ReceiverMode::Readonly));
        for method in members(&receiver, ByteMemberKind::Method) {
            assert!(!method.descriptor_metadata);
            if matches!(method.return_type, ResolvedType::ByteRegion(_)) {
                assert_eq!(
                    method.parameters.iter().map(|p| p.name).collect::<Vec<_>>(),
                    ["offset", "length"]
                );
                assert!(method
                    .parameters
                    .iter()
                    .all(|p| p.ty == ResolvedType::Integer(IntegerType::Int64)));
                assert!(method.accepts_named_arguments);
            }
        }
        let mutable = member(&receiver, "getWritableRegion");
        assert_eq!(mutable.is_some(), writable);
        if let Some(mutable) = mutable {
            assert_eq!(mutable.result_ownership(), BindingOwnership::WritableBorrow);
            assert_eq!(mutable.receiver_mode, Some(ReceiverMode::Writable));
        }
        let copy = member(&receiver, "copyFrom");
        assert_eq!(
            copy.is_some(),
            receiver == ResolvedType::ByteRegion(ByteRegionKind::Writable)
        );
        if let Some(copy) = copy {
            assert_eq!(copy.receiver_mode, Some(ReceiverMode::Writable));
            assert_eq!(copy.return_type, ResolvedType::Integer(IntegerType::Int64));
            assert_eq!(
                copy.parameters.iter().map(|p| &p.ty).collect::<Vec<_>>(),
                [&ResolvedType::ByteRegion(ByteRegionKind::Readonly)]
            );
            assert_eq!(copy.result_ownership(), BindingOwnership::Owned);
        }
        let within = member(&receiver, "copyWithin");
        assert_eq!(
            within.is_some(),
            receiver == ResolvedType::ByteRegion(ByteRegionKind::Writable)
        );
        if let Some(within) = within {
            assert_eq!(within.receiver_mode, Some(ReceiverMode::Writable));
            assert_eq!(within.return_type, ResolvedType::Void);
            assert_eq!(
                within.parameters.iter().map(|p| p.name).collect::<Vec<_>>(),
                ["offset", "length", "destination"]
            );
        }
        let length = member(&receiver, "length").unwrap();
        assert_eq!(length.kind, ByteMemberKind::Property);
        assert!(!length.writable_place);
        assert!(length.descriptor_metadata);
        assert_eq!(
            length.return_type,
            ResolvedType::Integer(IntegerType::Int64)
        );
        let index = member(&receiver, "[]").unwrap();
        assert_eq!(index.kind, ByteMemberKind::Index);
        assert_eq!(index.writable_place, writable);
        assert!(!index.descriptor_metadata);
        assert_eq!(index.return_type, ResolvedType::Integer(IntegerType::UInt8));
        if receiver != ResolvedType::Bytes {
            assert!(members(&receiver, ByteMemberKind::StaticMethod).is_empty());
            assert!(member(&receiver, "toArray").is_none());
        }
    }
}

#[test]
fn byte_metadata_recognizes_only_exact_compiler_identities() {
    for kind in [ByteRegionKind::Readonly, ByteRegionKind::Writable] {
        assert_eq!(
            type_from_source_name(kind.source_name()),
            Some(ResolvedType::ByteRegion(kind))
        );
    }
    assert!(type_from_source_name("ByteRegion").is_none());
    assert!(type_from_source_name("App\\Bytes").is_none());
    assert!(member(&ResolvedType::String, "length").is_none());
    assert!(member(
        &ResolvedType::Nullable(Box::new(ResolvedType::Bytes)),
        "getRegion"
    )
    .is_none());
}
