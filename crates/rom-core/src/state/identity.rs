use std::path::PathBuf;

/// Store path representation
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct StorePath {
  pub path: PathBuf,
  pub hash: String,
  pub name: String,
}

impl StorePath {
  #[must_use]
  pub fn parse(path: &str) -> Option<Self> {
    if !path.starts_with("/nix/store/") {
      return None;
    }

    let path_buf = PathBuf::from(path);
    let file_name = path_buf.file_name()?.to_str()?.to_string();

    let parts: Vec<&str> = file_name.splitn(2, '-').collect();
    if parts.len() != 2 {
      return None;
    }

    Some(Self {
      path: path_buf,
      hash: parts[0].to_string(),
      name: parts[1].to_string(),
    })
  }
}

/// Derivation representation
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Derivation {
  pub path: PathBuf,
  pub name: String,
}

impl Derivation {
  #[must_use]
  pub fn parse(path: &str) -> Option<Self> {
    // This is a trust boundary: accepted values are handed to the asynchronous
    // graph indexer and opened from the filesystem.  Require the lexical Nix
    // store-path form rather than merely looking at the final extension.
    let file_name = path.strip_prefix("/nix/store/")?;
    if file_name.is_empty() || file_name.contains('/') {
      return None;
    }
    let stem = file_name.strip_suffix(".drv")?;
    let (hash, display_name) = stem.split_once('-')?;
    const NIX_BASE32: &str = "0123456789abcdfghijklmnpqrsvwxyz";
    if hash.len() != 32
      || !hash
        .bytes()
        .all(|byte| NIX_BASE32.as_bytes().contains(&byte))
      || display_name.is_empty()
    {
      return None;
    }

    Some(Self {
      path: PathBuf::from(path),
      name: display_name.to_string(),
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn store_path_parse_splits_hash_and_name() {
    let path = "/nix/store/abc123-hello-1.0";
    let store_path = StorePath::parse(path).unwrap();

    assert_eq!(store_path.hash, "abc123");
    assert_eq!(store_path.name, "hello-1.0");
  }

  #[test]
  fn derivation_parse_uses_display_name_without_store_hash() {
    let path = "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-hello-1.0.drv";
    let derivation = Derivation::parse(path).unwrap();

    assert_eq!(derivation.name, "hello-1.0");
  }

  #[test]
  fn derivation_parse_rejects_noncanonical_and_arbitrary_paths() {
    for path in [
      "/tmp/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-hello.drv",
      "/nix/store/short-hello.drv",
      "/nix/store/eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee-hello.drv",
      "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-.drv",
      "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-dir/hello.drv",
      "/nix/store/../aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-hello.drv",
    ] {
      assert!(Derivation::parse(path).is_none(), "accepted {path}");
    }
  }

  #[cfg(unix)]
  #[test]
  fn derivation_parse_rejects_fifo_before_any_filesystem_read() {
    let directory = tempfile::tempdir().unwrap();
    let fifo = directory
      .path()
      .join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-block.drv");
    assert!(
      std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .unwrap()
        .success()
    );
    assert!(Derivation::parse(fifo.to_str().unwrap()).is_none());
  }
}
