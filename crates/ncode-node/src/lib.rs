//! Node.js bindings for Ncode via napi-rs.
//!
//! The native layer is deliberately thin: it owns the async connection and
//! returns query results as the raw NCB byte buffer (handed to V8 as an
//! external `Buffer`, so no copy on the way out). The fluent query builder and
//! the zero-copy columnar reader live in the TypeScript wrapper (`ts/`), which
//! is the idiomatic place for them in the Node ecosystem.
//!
//! Every Rust `async fn` below is surfaced to JavaScript as a `Promise` — the
//! napi-rs `tokio_rt` runtime drives the core's `Future`s.

use std::ffi::c_void;
use std::sync::Arc;

use napi::bindgen_prelude::*;
use napi_derive::napi;

use ncode_core::{Client as CoreClient, Value};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// An NCB buffer shared with the core's query cache, surfaced to V8 as an
/// *external* Buffer aliasing the cached bytes — no copy at any size. The
/// `Arc` travels in the finalize hint, so the bytes outlive every JS view.
/// The reader API only reads; mutating the raw Buffer is out of contract.
pub struct NcbBuffer(Arc<Vec<u8>>);

impl ToNapiValue for NcbBuffer {
    unsafe fn to_napi_value(env: sys::napi_env, val: Self) -> Result<sys::napi_value> {
        let len = val.0.len();
        let mut result = std::ptr::null_mut();
        if len == 0 {
            let mut data = std::ptr::null_mut();
            check_status!(
                sys::napi_create_buffer(env, 0, &mut data, &mut result),
                "failed to create empty buffer"
            )?;
            return Ok(result);
        }

        unsafe extern "C" fn finalize(_env: sys::napi_env, _data: *mut c_void, hint: *mut c_void) {
            drop(Box::from_raw(hint as *mut Arc<Vec<u8>>));
        }

        let data = val.0.as_ptr() as *mut c_void;
        let hint = Box::into_raw(Box::new(val.0)) as *mut c_void;
        let status = sys::napi_create_external_buffer(env, len, data, Some(finalize), hint, &mut result);
        if status != sys::Status::napi_ok {
            drop(Box::from_raw(hint as *mut Arc<Vec<u8>>));
            return Err(Error::new(
                Status::GenericFailure,
                format!("failed to create external buffer (status {status})"),
            ));
        }
        Ok(result)
    }
}

/// Convert a core error into a JS-friendly napi error.
fn map_err(e: ncode_core::Error) -> Error {
    Error::from_reason(e.to_string())
}

/// A single bound parameter arriving from JS: number | string | boolean | null.
type ParamValue = Either4<f64, String, bool, Null>;

fn to_value(p: ParamValue) -> Value {
    match p {
        // JS has only `number`; treat exact integers in the safe range as ints.
        Either4::A(n) => {
            if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
                Value::Int(n as i64)
            } else {
                Value::Float(n)
            }
        }
        Either4::B(s) => Value::Text(s),
        Either4::C(b) => Value::Bool(b),
        Either4::D(_) => Value::Null,
    }
}

fn to_values(params: Option<Vec<ParamValue>>) -> Vec<Value> {
    params.unwrap_or_default().into_iter().map(to_value).collect()
}

/// An async database client. Returned by [`connect`].
#[napi]
pub struct Client {
    inner: CoreClient,
}

/// Open a connection. Returns a `Promise<Client>`.
#[napi]
pub async fn connect(url: String) -> Result<Client> {
    let inner = CoreClient::connect(&url).await.map_err(map_err)?;
    Ok(Client { inner })
}

#[napi]
impl Client {
    /// Run a non-row statement (INSERT/UPDATE/DDL). Resolves to rows affected.
    #[napi]
    pub async fn execute(&self, sql: String, params: Option<Vec<ParamValue>>) -> Result<u32> {
        let n = self
            .inner
            .execute(&sql, to_values(params))
            .await
            .map_err(map_err)?;
        Ok(n as u32)
    }

    /// Run a query. Resolves to a `Buffer` holding the NCB columnar payload,
    /// which `decodeBatch()` in the TS wrapper turns into zero-copy typed
    /// arrays. The bytes are shared with the core's query cache and handed to
    /// V8 as an external buffer — no copy on the way out.
    #[napi]
    pub async fn query(&self, sql: String, params: Option<Vec<ParamValue>>) -> Result<NcbBuffer> {
        let bytes = self
            .inner
            .query_bytes_shared(&sql, to_values(params))
            .await
            .map_err(map_err)?;
        Ok(NcbBuffer(bytes))
    }

    /// Synchronous query-cache probe: returns the cached NCB buffer for an
    /// identical prior query on an unchanged `:memory:` database, or `null`.
    /// A hit skips the async bridge, the thread hop, the scan, and the encode
    /// entirely — this is the microsecond path for repeated reads.
    #[napi]
    pub fn query_cached(&self, sql: String, params: Option<Vec<ParamValue>>) -> Option<NcbBuffer> {
        self.inner.probe_cache(&sql, to_values(params)).map(NcbBuffer)
    }
}
