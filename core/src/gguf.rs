//! Read the small amount of GGUF metadata needed for placement. Tensor data and
//! tokenizer arrays are skipped, so inspecting a model does not load its weights.

use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::io::{BufReader, Read, Seek};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct Metadata {
    pub architecture: String,
    pub tensor_count: u64,
    pub layers: u32,
    pub kv_bytes_per_token: Option<u64>,
    pub params_b: Option<f64>,
    pub split_no: Option<u64>,
    pub split_count: Option<u64>,
    pub layer_bytes: Vec<u64>,
    pub coordinator_bytes: u64,
    pub metadata_bytes: u64,
    pub max_context: Option<u32>,
}

pub fn inspect(path: &Path) -> Result<Metadata> {
    let file =
        std::fs::File::open(path).with_context(|| format!("open model {}", path.display()))?;
    let length = file.metadata()?.len();
    let mut reader = Reader {
        inner: BufReader::new(file),
        limit: length.min(128 * 1024 * 1024),
    };
    if reader.bytes::<4>()? != *b"GGUF" {
        bail!("{} is not a GGUF model", path.display());
    }
    let version = reader.u32()?;
    if !matches!(version, 2 | 3) {
        bail!("unsupported GGUF version {version}");
    }
    let tensor_count = reader.u64()?;
    if tensor_count == 0 || tensor_count > 10_000_000 {
        bail!("GGUF tensor count is missing or invalid");
    }
    let count = reader.u64()?;
    if count > 100_000 {
        bail!("GGUF metadata count is too large");
    }
    let mut architecture = None;
    let mut numbers = HashMap::new();
    for _ in 0..count {
        let key = reader.string(1024)?;
        let kind = reader.u32()?;
        if key == "general.architecture" {
            if kind != 8 {
                bail!("GGUF architecture is not a string");
            }
            architecture = Some(reader.string(256)?);
        } else if key == "general.parameter_count"
            || key == "general.alignment"
            || key == "split.no"
            || key == "split.count"
            || key.ends_with(".block_count")
            || key.ends_with(".embedding_length")
            || key.ends_with(".attention.head_count")
            || key.ends_with(".attention.head_count_kv")
            || key.ends_with(".attention.key_length")
            || key.ends_with(".attention.value_length")
            || key.ends_with(".context_length")
        {
            if let Some(number) = reader.number(kind)? {
                numbers.insert(key, number);
            }
        } else {
            reader.skip_value(kind, 0)?;
        }
    }
    let architecture = architecture.context("GGUF architecture is missing")?;
    let get = |suffix: &str| numbers.get(&format!("{architecture}.{suffix}")).copied();
    let layers = get("block_count")
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n > 0 && *n <= 4096)
        .context("GGUF transformer block count is missing or invalid")?;
    let kv_bytes_per_token = (|| {
        let heads = get("attention.head_count")?;
        let kv_heads = get("attention.head_count_kv").unwrap_or(heads);
        let hidden = get("embedding_length")?;
        if heads == 0 || kv_heads == 0 || hidden % heads != 0 {
            return None;
        }
        let keys = get("attention.key_length").unwrap_or(hidden / heads);
        let values = get("attention.value_length").unwrap_or(hidden / heads);
        // The managed launch uses f16 K/V caches: two bytes per value.
        u64::from(layers)
            .checked_mul(kv_heads)?
            .checked_mul(keys.checked_add(values)?)?
            .checked_mul(2)
    })();
    let alignment = numbers.get("general.alignment").copied().unwrap_or(32);
    if alignment == 0 || !alignment.is_power_of_two() || alignment > 4096 {
        bail!("GGUF tensor alignment is invalid");
    }
    let mut tensors = Vec::with_capacity(tensor_count.min(100_000) as usize);
    let mut names = std::collections::HashSet::new();
    let mut layer_bytes = vec![0u64; layers as usize];
    let mut coordinator_bytes = 0u64;
    for _ in 0..tensor_count {
        let name = reader.string(4096)?;
        if !names.insert(name.clone()) {
            bail!("GGUF tensor names must be unique");
        }
        let dimensions = reader.u32()?;
        if dimensions == 0 || dimensions > 4 {
            bail!("GGUF tensor dimensions are invalid");
        }
        let mut elements = 1u128;
        let mut row = 0u64;
        for index in 0..dimensions {
            let dimension = reader.u64()?;
            if dimension == 0 {
                bail!("GGUF tensor has an empty dimension");
            }
            elements = elements
                .checked_mul(u128::from(dimension))
                .context("GGUF tensor dimensions overflow")?;
            if index == 0 {
                row = dimension;
            }
        }
        let kind = reader.u32()?;
        let (block, bytes) =
            tensor_layout(kind).context("GGUF tensor type is unsupported by the pinned runtime")?;
        if !row.is_multiple_of(block) {
            bail!("GGUF quantized tensor row is not aligned to its block size");
        }
        let tensor_bytes = elements
            .checked_div(u128::from(block))
            .and_then(|blocks| blocks.checked_mul(u128::from(bytes)))
            .and_then(|bytes| u64::try_from(bytes).ok())
            .context("GGUF tensor type or size is unsupported")?;
        let offset = reader.u64()?;
        if !offset.is_multiple_of(alignment) {
            bail!("GGUF tensor offset is not aligned");
        }
        tensors.push((offset, tensor_bytes));
        if let Some(index) = name
            .strip_prefix("blk.")
            .and_then(|rest| rest.split('.').next())
            .and_then(|value| value.parse::<usize>().ok())
        {
            let bytes = layer_bytes
                .get_mut(index)
                .context("GGUF tensor references an invalid transformer block")?;
            *bytes = bytes
                .checked_add(tensor_bytes)
                .context("GGUF block size overflow")?;
        } else {
            coordinator_bytes = coordinator_bytes
                .checked_add(tensor_bytes)
                .context("GGUF coordinator weight size overflow")?;
        }
    }
    let tensor_info_end = reader.position()?;
    let data_start = tensor_info_end
        .checked_add(alignment - 1)
        .map(|value| value / alignment * alignment)
        .context("GGUF data offset overflow")?;
    if data_start >= length
        || tensors.iter().any(|(offset, bytes)| {
            data_start
                .checked_add(*offset)
                .and_then(|start| start.checked_add(*bytes))
                .is_none_or(|end| end > length)
        })
    {
        bail!("GGUF tensor data is missing or outside the file");
    }
    let max_context = get("context_length")
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0);
    Ok(Metadata {
        architecture,
        tensor_count,
        layers,
        kv_bytes_per_token,
        params_b: numbers
            .get("general.parameter_count")
            .map(|n| *n as f64 / 1e9),
        split_no: numbers.get("split.no").copied(),
        split_count: numbers.get("split.count").copied(),
        layer_bytes,
        coordinator_bytes,
        metadata_bytes: data_start,
        max_context,
    })
}

/// GGML on-disk layouts from llama.cpp b12110. Runtime-only/deprecated types
/// are intentionally excluded; block overhead is included in every extent.
fn tensor_layout(kind: u32) -> Option<(u64, u64)> {
    Some(match kind {
        0 => (1, 4),
        1 => (1, 2),
        2 => (32, 18),
        3 => (32, 20),
        6 => (32, 22),
        7 => (32, 24),
        8 => (32, 34),
        9 => (32, 36),
        10 => (256, 84),
        11 => (256, 110),
        12 => (256, 144),
        13 => (256, 176),
        14 => (256, 210),
        15 => (256, 292),
        16 => (256, 66),
        17 => (256, 74),
        18 => (256, 98),
        19 => (256, 50),
        20 => (32, 18),
        21 => (256, 110),
        22 => (256, 82),
        23 => (256, 136),
        24 => (1, 1),
        25 => (1, 2),
        26 => (1, 4),
        27 | 28 => (1, 8),
        29 => (256, 56),
        30 => (1, 2),
        34 => (256, 54),
        35 => (256, 66),
        39 => (32, 17),
        40 => (64, 36),
        41 => (128, 18),
        42 => (64, 18),
        43 => (1, 1),
        _ => return None,
    })
}

/// llama.cpp's standard shard naming. The numeric index is one-based in the
/// filename and zero-based in GGUF metadata.
pub fn split_name(filename: &str) -> Option<(&str, u32, u32)> {
    let stem = filename.strip_suffix(".gguf")?;
    let (left, count) = stem.rsplit_once("-of-")?;
    let (prefix, part) = left.rsplit_once('-')?;
    if prefix.is_empty() || part.len() != 5 || count.len() != 5 {
        return None;
    }
    let (part, count): (u32, u32) = (part.parse().ok()?, count.parse().ok()?);
    (part > 0 && part <= count).then_some((prefix, part, count))
}

/// Validate the complete shard set and count every weight file for planning.
/// Passing a later shard as the entry point must not understate model size.
pub fn model_files(path: &Path, metadata: &Metadata) -> Result<Vec<PathBuf>> {
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("invalid GGUF filename")?;
    let named_split = split_name(filename);
    let count = metadata.split_count.unwrap_or(1);
    if count <= 1 {
        if named_split.is_some_and(|(_, _, count)| count > 1) {
            bail!("split GGUF is missing its shard metadata");
        }
        return Ok(vec![path.to_path_buf()]);
    }
    let (prefix, part, named_count) =
        named_split.context("split GGUF must use its original shard filenames")?;
    if part != 1 || metadata.split_no != Some(0) || count != u64::from(named_count) || count > 1024
    {
        bail!("select the first GGUF shard of a complete model (00001-of-NNNNN)");
    }
    let mut files = Vec::new();
    for index in 1..=named_count {
        let file = path.with_file_name(format!("{prefix}-{index:05}-of-{named_count:05}.gguf"));
        if index > 1 {
            let other = inspect(&file)
                .with_context(|| format!("missing or invalid GGUF shard {}", file.display()))?;
            if other.split_no != Some(u64::from(index - 1))
                || other.split_count != Some(count)
                || other.layers != metadata.layers
                || other.architecture != metadata.architecture
                || other.params_b != metadata.params_b
            {
                bail!("GGUF shards do not belong to the same model");
            }
        }
        files.push(file);
    }
    Ok(files)
}

struct Reader<R> {
    inner: R,
    limit: u64,
}

impl<R: Read + Seek> Reader<R> {
    fn position(&mut self) -> Result<u64> {
        Ok(self.inner.stream_position()?)
    }
    fn remaining(&mut self, length: u64) -> Result<()> {
        let end = self
            .inner
            .stream_position()?
            .checked_add(length)
            .context("GGUF offset overflow")?;
        if end > self.limit {
            bail!("GGUF metadata is truncated or exceeds 128 MiB");
        }
        Ok(())
    }

    fn bytes<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.remaining(N as u64)?;
        let mut bytes = [0; N];
        self.inner.read_exact(&mut bytes)?;
        Ok(bytes)
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes()?))
    }
    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.bytes()?))
    }

    fn string(&mut self, maximum: u64) -> Result<String> {
        let length = self.u64()?;
        if length > maximum {
            bail!("GGUF metadata string exceeds {maximum} bytes");
        }
        self.remaining(length)?;
        let mut bytes = vec![0; length as usize];
        self.inner.read_exact(&mut bytes)?;
        String::from_utf8(bytes).context("invalid UTF-8 in GGUF metadata")
    }

    fn skip(&mut self, length: u64) -> Result<()> {
        self.remaining(length)?;
        self.inner.seek_relative(i64::try_from(length)?)?;
        Ok(())
    }

    fn number(&mut self, kind: u32) -> Result<Option<u64>> {
        Ok(match kind {
            0 => Some(self.bytes::<1>()?[0] as u64),
            1 => u64::try_from(i8::from_le_bytes(self.bytes()?)).ok(),
            2 => Some(u16::from_le_bytes(self.bytes()?) as u64),
            3 => u64::try_from(i16::from_le_bytes(self.bytes()?)).ok(),
            4 => Some(self.u32()? as u64),
            5 => u64::try_from(i32::from_le_bytes(self.bytes()?)).ok(),
            10 => Some(self.u64()?),
            11 => u64::try_from(i64::from_le_bytes(self.bytes()?)).ok(),
            _ => {
                self.skip_value(kind, 0)?;
                None
            }
        })
    }

    fn skip_value(&mut self, kind: u32, depth: u32) -> Result<()> {
        if let Some(size) = scalar_size(kind) {
            return self.skip(size);
        }
        match kind {
            8 => {
                let length = self.u64()?;
                self.skip(length)
            }
            9 => {
                if depth >= 4 {
                    bail!("GGUF array nesting is too deep");
                }
                let element = self.u32()?;
                let count = self.u64()?;
                if count > 10_000_000 {
                    bail!("GGUF metadata array is too large");
                }
                if let Some(size) = scalar_size(element) {
                    return self.skip(size.checked_mul(count).context("GGUF array overflow")?);
                }
                for _ in 0..count {
                    self.skip_value(element, depth + 1)?;
                }
                Ok(())
            }
            _ => bail!("invalid GGUF metadata value type {kind}"),
        }
    }
}

fn scalar_size(kind: u32) -> Option<u64> {
    match kind {
        0 | 1 | 7 => Some(1),
        2 | 3 => Some(2),
        4..=6 => Some(4),
        10..=12 => Some(8),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) fn fixture(layers: u32) -> Vec<u8> {
    fixture_with_split(layers, None)
}

#[cfg(test)]
fn fixture_with_split(layers: u32, split: Option<(u32, u32)>) -> Vec<u8> {
    fn string(out: &mut Vec<u8>, value: &str) {
        out.extend_from_slice(&(value.len() as u64).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
    }
    let mut bytes = b"GGUF".to_vec();
    bytes.extend_from_slice(&3u32.to_le_bytes());
    let tensor_indices: Vec<_> = (0..layers.max(1))
        .filter(|index| split.is_none_or(|(part, count)| index % count == part))
        .collect();
    bytes.extend_from_slice(&(tensor_indices.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&(6u64 + if split.is_some() { 2 } else { 0 }).to_le_bytes());
    string(&mut bytes, "general.architecture");
    bytes.extend_from_slice(&8u32.to_le_bytes());
    string(&mut bytes, "qwen2");
    for (name, value) in [
        ("block_count", layers),
        ("embedding_length", 1536),
        ("attention.head_count", 12),
        ("attention.head_count_kv", 2),
    ] {
        string(&mut bytes, &format!("qwen2.{name}"));
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    string(&mut bytes, "tokenizer.ggml.tokens");
    bytes.extend_from_slice(&9u32.to_le_bytes());
    bytes.extend_from_slice(&8u32.to_le_bytes());
    bytes.extend_from_slice(&2u64.to_le_bytes());
    string(&mut bytes, "hello");
    string(&mut bytes, "world");
    if let Some((index, count)) = split {
        for (name, value) in [("split.no", index), ("split.count", count)] {
            string(&mut bytes, name);
            bytes.extend_from_slice(&4u32.to_le_bytes());
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    for (position, index) in tensor_indices.iter().enumerate() {
        string(&mut bytes, &format!("blk.{index}.weight"));
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&1u64.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&(position as u64 * 32).to_le_bytes());
    }
    while !bytes.len().is_multiple_of(32) {
        bytes.push(0);
    }
    for _ in 0..tensor_indices.len().saturating_sub(1) {
        bytes.extend_from_slice(&[0; 32]);
    }
    bytes.extend_from_slice(&0f32.to_le_bytes());
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shard(layers: u32, index: u32, count: u32) -> Vec<u8> {
        fixture_with_split(layers, Some((index, count)))
    }

    #[test]
    fn split_models_require_every_shard_and_plan_the_combined_size() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("model-00001-of-00002.gguf");
        let second = dir.path().join("model-00002-of-00002.gguf");
        std::fs::write(&first, shard(28, 0, 2)).unwrap();
        assert!(crate::plan::ModelInfo::from_file(&first, "chosen").is_err());
        std::fs::write(&second, shard(28, 1, 2)).unwrap();
        for path in [&first, &second] {
            std::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .unwrap()
                .set_len(1024 * 1024 + 1)
                .unwrap();
        }
        let model = crate::plan::ModelInfo::from_file(&first, "chosen").unwrap();
        assert_eq!(model.size_mb, 3);
        assert_eq!(model.layers, 28);
        assert!(crate::plan::ModelInfo::from_file(&second, "chosen").is_err());
        std::fs::write(&second, shard(32, 1, 2)).unwrap();
        assert!(crate::plan::ModelInfo::from_file(&first, "chosen").is_err());
    }

    #[test]
    fn reads_exact_block_count_and_default_f16_kv_size() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("model.gguf");
        std::fs::write(&path, fixture(28)).unwrap();
        let metadata = inspect(&path).unwrap();
        assert_eq!(metadata.layers, 28);
        assert_eq!(metadata.tensor_count, 28);
        assert_eq!(metadata.architecture, "qwen2");
        assert_eq!(metadata.kv_bytes_per_token, Some(28 * 2 * 128 * 4));
    }

    #[test]
    fn rejects_partial_and_non_gguf_files_without_loading_weights() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("model.gguf");
        for bytes in [
            b"<html>not a model</html>".to_vec(),
            fixture(0),
            fixture(28)[..32].to_vec(),
        ] {
            std::fs::write(&path, bytes).unwrap();
            assert!(inspect(&path).is_err());
        }
    }

    #[test]
    fn rejects_metadata_only_files_without_tensor_data() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("model.gguf");
        let mut bytes = fixture(28);
        bytes.truncate(bytes.len() - 4);
        std::fs::write(&path, bytes).unwrap();
        assert!(inspect(&path)
            .unwrap_err()
            .to_string()
            .contains("tensor data"));
    }

    #[test]
    fn rejects_a_tensor_whose_full_extent_is_truncated() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("model.gguf");
        let mut bytes = fixture(28);
        bytes.truncate(bytes.len() - 1);
        std::fs::write(&path, bytes).unwrap();
        assert!(inspect(&path)
            .unwrap_err()
            .to_string()
            .contains("tensor data"));
    }

    #[test]
    fn rejects_truncated_quantized_blocks_using_exact_type_extents() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("model.gguf");
        let mut bytes = fixture(1);
        // The fixture's only tensor descriptor ends 4 bytes before aligned F32
        // data. Change its dimension/type to one Q4_0 block but leave 4 bytes;
        // a valid block requires 18 bytes in llama.cpp b12110.
        let descriptor = bytes
            .windows("blk.0.weight".len())
            .position(|window| window == b"blk.0.weight")
            .unwrap();
        let after_name = descriptor + "blk.0.weight".len();
        let dimension = after_name + 4;
        bytes[dimension..dimension + 8].copy_from_slice(&32u64.to_le_bytes());
        bytes[dimension + 8..dimension + 12].copy_from_slice(&2u32.to_le_bytes());
        std::fs::write(&path, bytes).unwrap();
        assert!(inspect(&path)
            .unwrap_err()
            .to_string()
            .contains("tensor data"));
    }
}
