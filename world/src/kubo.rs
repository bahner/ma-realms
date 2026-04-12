//! Compatibility shim — re-exports from ma_core::kubo with legacy names.

pub use ma_core::kubo::{
    IpnsPublishOptions, fetch_did_document, generate_key as generate_kubo_key,
    import_key as import_kubo_key, ipfs_add, list_key_names as list_kubo_key_names,
    list_keys as list_kubo_keys, name_publish_with_retry as ipns_publish_with_retry,
    name_resolve, pin_add_named, pin_rm, wait_for_api as wait_for_kubo_api,
    dag_put as dag_put_dag_cbor, dag_get as dag_get_dag_cbor,
    cat_text as cat_cid,
};
