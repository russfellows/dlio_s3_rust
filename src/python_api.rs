// src/python_api.rs
//! PyO3 bindings – expose the same operations as the Rust CLI.

#![allow(unsafe_op_in_unsafe_fn)]

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyAny};
use pyo3::exceptions::PyRuntimeError;
use regex::Regex;

use crate::s3_utils::{
    delete_objects, get_object_uri, get_objects_parallel, list_objects, parse_s3_uri,
    put_objects_parallel, generate_random_data, DEFAULT_OBJECT_SIZE, create_bucket,
    //put_object, put_objects_parallel, generate_random_data, DEFAULT_OBJECT_SIZE, create_bucket,
    // Note: the put_object is never called now, we always call put_objects_parallel
};

/// Convert an error into a Python RuntimeError.
fn py_err<E: std::fmt::Display>(e: E) -> PyErr {
    PyRuntimeError::new_err(e.to_string())
}


/*
 * Old Put
 *
/// `put(uris, max_in_flight=32, size=None, create_bucket=False) -> None`
/// Uploads multiple objects concurrently with random data (default 20 MB).
/// This new put, is the old put_many, but will handle a single object as well
#[pyfunction]
#[pyo3(signature = (uris, max_in_flight = 32, size = None, should_create_bucket = false))]
pub fn put(uris: Vec<String>, max_in_flight: usize, size: Option<usize>, should_create_bucket: bool) -> PyResult<()> {
    let size = size.unwrap_or(DEFAULT_OBJECT_SIZE);
    if !uris.is_empty() {
        let (bucket, _) = parse_s3_uri(&uris[0]).map_err(py_err)?;
        if should_create_bucket {
            if let Err(e) = create_bucket(&bucket) {
                eprintln!("Warning: failed to create bucket {}: {}", bucket, e);
            }
        }
    }
    let data = generate_random_data(size);
    put_objects_parallel(&uris, &data, max_in_flight).map_err(py_err)
}
*/

/// `put(prefix, num=1, template="object_{}.dat", max_in_flight=32, size=None, should_create_bucket=False) -> None`
/// Uploads one or many objects with random data (default 20 MB). The `prefix` is an S3 URI
/// that specifies the bucket and key prefix. The function generates object names using the provided `template`.
#[pyfunction]
#[pyo3(signature = (prefix, num=1, template="object_{}.dat", max_in_flight=32, size=None, should_create_bucket=false))]
pub fn put(prefix: &str, num: usize, template: &str, max_in_flight: usize, size: Option<usize>, should_create_bucket: bool) -> PyResult<()> {
    let size = size.unwrap_or(DEFAULT_OBJECT_SIZE);
    let (bucket, mut key_prefix) = parse_s3_uri(prefix).map_err(py_err)?;
    if should_create_bucket {
        if let Err(e) = create_bucket(&bucket) {
            eprintln!("Warning: failed to create bucket {}: {}", bucket, e);
        }
    }
    if !key_prefix.ends_with('/') {
        key_prefix.push('/');
    }
    // Generate the list of full URIs to be uploaded.
    let mut uris = Vec::with_capacity(num);
    for i in 0..num {
        let object_name = template.replace("{}", &i.to_string());
        let full_uri = format!("s3://{}/{}{}", bucket, key_prefix, object_name);
        uris.push(full_uri);
    }
    let data = generate_random_data(size);
    // Compute effective concurrency: use the smaller of max_in_flight and the number of URIs.
    let effective_jobs = std::cmp::min(max_in_flight, num);
    put_objects_parallel(&uris, &data, effective_jobs).map_err(py_err)
}

/// `list("s3://bucket/pattern") -> [str, ...]`
/// Lists objects matching the given pattern. Supports wildcard '*' in the key.
#[pyfunction]
pub fn list(uri: &str) -> PyResult<Vec<String>> {
    let (bucket, key_pattern) = parse_s3_uri(uri).map_err(py_err)?;
    if key_pattern.contains('*') {
        let (effective_prefix, glob_pattern) = if let Some(pos) = key_pattern.rfind('/') {
            (&key_pattern[..=pos], &key_pattern[pos+1..])
        } else {
            ("", key_pattern.as_str())
        };
        let mut keys = list_objects(&bucket, effective_prefix).map_err(py_err)?;
        if glob_pattern.contains('*') {
            let regex_pattern = format!("^{}$", regex::escape(glob_pattern).replace("\\*", ".*"));
            let re = Regex::new(&regex_pattern).map_err(py_err)?;
            keys = keys.into_iter()
                .filter(|k| {
                    let basename = k.rsplit('/').next().unwrap_or(k);
                    re.is_match(basename)
                })
                .collect();
        }
        Ok(keys)
    } else {
        list_objects(&bucket, &key_pattern).map_err(py_err)
    }
}

/// `get("s3://bucket/key_or_pattern") -> bytes`
/// Downloads a single object. If the key contains a wildcard, returns the first matching object.
#[pyfunction]
pub fn get(py: Python<'_>, uri: &str) -> PyResult<Py<PyBytes>> {
    let (bucket, key_pattern) = parse_s3_uri(uri).map_err(py_err)?;
    if key_pattern.contains('*') {
        let (effective_prefix, glob_pattern) = if let Some(pos) = key_pattern.rfind('/') {
            (&key_pattern[..=pos], &key_pattern[pos+1..])
        } else {
            ("", key_pattern.as_str())
        };
        let mut keys = list_objects(&bucket, effective_prefix).map_err(py_err)?;
        if glob_pattern.contains('*') {
            let regex_pattern = format!("^{}$", regex::escape(glob_pattern).replace("\\*", ".*"));
            let re = Regex::new(&regex_pattern).map_err(py_err)?;
            keys = keys.into_iter()
                .filter(|k| {
                    let basename = k.rsplit('/').next().unwrap_or(k);
                    re.is_match(basename)
                })
                .collect();
        }
        if let Some(key) = keys.first() {
            let full_uri = format!("s3://{}/{}", bucket, key);
            let bytes = get_object_uri(&full_uri).map_err(py_err)?;
            Ok(PyBytes::new(py, &bytes).into())
        } else {
            Err(py_err("No matching object found"))
        }
    } else if key_pattern.ends_with('/') || key_pattern.is_empty() {
        Err(py_err("Bulk get not supported in get(). Use get_many() instead."))
    } else {
        let full_uri = format!("s3://{}/{}", bucket, key_pattern);
        let bytes = get_object_uri(&full_uri).map_err(py_err)?;
        Ok(PyBytes::new(py, &bytes).into())
    }
}

/// `get_many(uris, max_in_flight=64) -> [(str, bytes), ...]`
/// Downloads multiple objects concurrently.
#[pyfunction]
#[pyo3(signature = (uris, max_in_flight = 64))]
pub fn get_many(py: Python<'_>, uris: Vec<String>, max_in_flight: usize) -> PyResult<Vec<(String, Py<PyBytes>)>> {
    let res = get_objects_parallel(&uris, max_in_flight).map_err(py_err)?;
    Ok(res.into_iter().map(|(u, b)| (u, PyBytes::new(py, &b).into())).collect())
}

/// `delete("s3://bucket/key_or_prefix") -> None`
/// Deletes an object or all objects under a prefix.
#[pyfunction]
pub fn delete(uri: &str) -> PyResult<()> {
    let (bucket, key) = parse_s3_uri(uri).map_err(py_err)?;
    let keys = if key.ends_with('/') || key.is_empty() {
        list_objects(&bucket, &key).map_err(py_err)?
    } else {
        vec![key]
    };
    delete_objects(&bucket, &keys).map_err(py_err)
}

/// Optional NPZ helper.
use numpy::PyArrayDyn;
use std::io::Cursor;

#[pyfunction]
#[pyo3(signature = (uri, array_name = None))]
pub fn read_npz(py: Python<'_>, uri: &str, array_name: Option<&str>) -> PyResult<Py<PyAny>> {
    let bytes = get_object_uri(uri).map_err(py_err)?;
    let cursor = Cursor::new(bytes);
    let mut npz = ndarray_npy::NpzReader::new(cursor).map_err(py_err)?;

    let name = array_name
        .map(|s| s.to_owned())
        .or_else(|| npz.names().ok().and_then(|mut v| v.pop()))
        .ok_or_else(|| PyRuntimeError::new_err("NPZ file is empty"))?;

    let arr: ndarray::ArrayD<f32> = npz.by_name(&name).map_err(py_err)?;
    let py_arr = PyArrayDyn::<f32>::from_owned_array(py, arr);
    Ok(py_arr.into_py(py))
}

