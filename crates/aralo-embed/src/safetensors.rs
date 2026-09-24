//! Just enough of the safetensors format to read a static embedding model.
//!
//! A safetensors file is an eight-byte little-endian header length, a JSON
//! header naming each tensor's type, shape and byte range, and the bytes. A
//! model here is one matrix and two optional vectors, so a reader of the
//! header and four element types is all this needs.

use std::collections::HashMap;

use serde_json::Value;

use crate::ModelError;

/// One tensor's description, pointing into the file it came from.
#[derive(Debug)]
pub(crate) struct Tensor<'a> {
    pub(crate) shape: Vec<usize>,
    dtype: String,
    data: &'a [u8],
}

pub(crate) fn read(bytes: &[u8]) -> Result<HashMap<String, Tensor<'_>>, ModelError> {
    let length: [u8; 8] = bytes
        .get(..8)
        .and_then(|head| head.try_into().ok())
        .ok_or_else(|| bad("the file is too short to be safetensors"))?;
    let length = usize::try_from(u64::from_le_bytes(length))
        .map_err(|_| bad("the header is longer than this machine can address"))?;
    let end = 8usize
        .checked_add(length)
        .filter(|&end| end <= bytes.len())
        .ok_or_else(|| bad("the header runs past the end of the file"))?;
    let header: HashMap<String, Value> =
        serde_json::from_slice(&bytes[8..end]).map_err(|error| bad(&error.to_string()))?;
    let data = &bytes[end..];

    let mut tensors = HashMap::new();
    for (name, info) in header {
        if name == "__metadata__" {
            continue;
        }
        let dtype = info["dtype"].as_str().unwrap_or_default().to_owned();
        let shape = info["shape"]
            .as_array()
            .ok_or_else(|| bad(&format!("{name} has no shape")))?
            .iter()
            .map(|size| size.as_u64().and_then(|size| usize::try_from(size).ok()))
            .collect::<Option<Vec<usize>>>()
            .ok_or_else(|| bad(&format!("{name} has a shape that is not a list of sizes")))?;
        let range = info["data_offsets"]
            .as_array()
            .filter(|range| range.len() == 2)
            .and_then(|range| {
                let start = usize::try_from(range[0].as_u64()?).ok()?;
                let end = usize::try_from(range[1].as_u64()?).ok()?;
                (start <= end && end <= data.len()).then_some(start..end)
            })
            .ok_or_else(|| bad(&format!("{name} points outside the file")))?;
        let tensor = Tensor {
            shape,
            dtype,
            data: &data[range],
        };
        let expected = tensor
            .shape
            .iter()
            .try_fold(width(&tensor.dtype), |bytes, &size| bytes.checked_mul(size));
        if expected != Some(tensor.data.len()) {
            return Err(bad(&format!(
                "{name} holds {} bytes, which is not a {} {:?}",
                tensor.data.len(),
                tensor.dtype,
                tensor.shape
            )));
        }
        tensors.insert(name, tensor);
    }
    Ok(tensors)
}

impl Tensor<'_> {
    /// The elements as `f32`. Integers are taken at face value: the model's
    /// output is scaled to unit length, so a matrix stored as `I8` with one
    /// scale for all of it needs the scale no more than the reference does.
    pub(crate) fn floats(&self) -> Result<Vec<f32>, ModelError> {
        let data = self.data;
        Ok(match self.dtype.as_str() {
            "F32" => data
                .chunks_exact(4)
                .map(|bytes| f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
                .collect(),
            "F64" => data
                .chunks_exact(8)
                .map(|bytes| {
                    let mut eight = [0; 8];
                    eight.copy_from_slice(bytes);
                    f64::from_le_bytes(eight) as f32
                })
                .collect(),
            "F16" => data
                .chunks_exact(2)
                .map(|bytes| half(u16::from_le_bytes([bytes[0], bytes[1]])))
                .collect(),
            "I8" => data.iter().map(|&byte| f32::from(byte as i8)).collect(),
            other => return Err(bad(&format!("{other} values cannot be read as numbers"))),
        })
    }

    /// The elements as indices.
    pub(crate) fn indices(&self) -> Result<Vec<u32>, ModelError> {
        let data = self.data;
        let wide = |value: i64| u32::try_from(value).map_err(|_| bad("an index is out of range"));
        match self.dtype.as_str() {
            "I32" => data
                .chunks_exact(4)
                .map(|bytes| {
                    wide(i64::from(i32::from_le_bytes([
                        bytes[0], bytes[1], bytes[2], bytes[3],
                    ])))
                })
                .collect(),
            "I64" => data
                .chunks_exact(8)
                .map(|bytes| {
                    let mut eight = [0; 8];
                    eight.copy_from_slice(bytes);
                    wide(i64::from_le_bytes(eight))
                })
                .collect(),
            other => Err(bad(&format!("{other} values cannot be read as indices"))),
        }
    }
}

/// Bytes per element, or zero for a type this reader does not know, which then
/// fails the size check.
fn width(dtype: &str) -> usize {
    match dtype {
        "I8" | "U8" | "BOOL" => 1,
        "F16" | "BF16" | "I16" | "U16" => 2,
        "F32" | "I32" | "U32" => 4,
        "F64" | "I64" | "U64" => 8,
        _ => 0,
    }
}

/// An IEEE half-precision number, widened.
fn half(bits: u16) -> f32 {
    let sign = u32::from(bits >> 15) << 31;
    let exponent = u32::from((bits >> 10) & 0x1f);
    let mantissa = u32::from(bits & 0x3ff);
    let widened = match exponent {
        0 if mantissa == 0 => sign,
        0 => {
            // Subnormal: the mantissa counts units of 2^-24.
            let value = mantissa as f32 / 16_777_216.0;
            return if sign == 0 { value } else { -value };
        }
        0x1f => sign | 0x7f80_0000 | (mantissa << 13),
        _ => sign | ((exponent + 112) << 23) | (mantissa << 13),
    };
    f32::from_bits(widened)
}

fn bad(reason: &str) -> ModelError {
    ModelError::Tensors(reason.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_precision_widens_exactly() {
        assert_eq!(half(0x3c00), 1.0);
        assert_eq!(half(0xc000), -2.0);
        assert_eq!(half(0x3555), 0.333_251_95);
        assert_eq!(half(0x0000), 0.0);
        assert_eq!(half(0x0001), 5.960_464_5e-8);
        assert_eq!(half(0x7bff), 65504.0);
        assert!(half(0x7c00).is_infinite());
    }

    fn file(header: &str, data: &[u8]) -> Vec<u8> {
        let mut bytes = (header.len() as u64).to_le_bytes().to_vec();
        bytes.extend_from_slice(header.as_bytes());
        bytes.extend_from_slice(data);
        bytes
    }

    #[test]
    fn a_tensor_is_read_from_its_range() {
        let data: Vec<u8> = [1.5f32, -2.0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        let bytes = file(
            r#"{"__metadata__":{"format":"pt"},"v":{"dtype":"F32","shape":[2],"data_offsets":[0,8]}}"#,
            &data,
        );
        let tensors = read(&bytes).unwrap();
        assert_eq!(tensors["v"].shape, vec![2]);
        assert_eq!(tensors["v"].floats().unwrap(), vec![1.5, -2.0]);
    }

    #[test]
    fn a_range_that_does_not_fit_its_shape_is_refused() {
        let bytes = file(
            r#"{"v":{"dtype":"F32","shape":[3],"data_offsets":[0,8]}}"#,
            &[0; 8],
        );
        assert!(read(&bytes).is_err());
        let bytes = file(
            r#"{"v":{"dtype":"F32","shape":[4],"data_offsets":[0,16]}}"#,
            &[0; 8],
        );
        assert!(read(&bytes).is_err());
        assert!(read(&[1, 2, 3]).is_err());
    }
}
