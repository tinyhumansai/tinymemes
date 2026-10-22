use super::*;
use tinystoragedrivers_types::{Capability, ErrorKind};

#[test]
fn every_native_storage_kind_maps_to_its_contract_mirror() {
    let cases = [
        (ErrorKind::NotFound, MarkerStorageKind::NotFound),
        (ErrorKind::Conflict, MarkerStorageKind::Conflict),
        (ErrorKind::AlreadyExists, MarkerStorageKind::AlreadyExists),
        (
            ErrorKind::Unsupported(Capability::ValidatedBatch),
            MarkerStorageKind::Unsupported,
        ),
        (ErrorKind::InvalidInput, MarkerStorageKind::InvalidInput),
        (ErrorKind::Serialization, MarkerStorageKind::Serialization),
        (ErrorKind::Crypto, MarkerStorageKind::Crypto),
        (ErrorKind::Unavailable, MarkerStorageKind::Unavailable),
        (ErrorKind::Backend, MarkerStorageKind::Backend),
    ];
    for (native, contract) in cases {
        assert_eq!(marker_kind(native), contract);
    }
}
