//! What local recognition can download: the speech models (GGUF files from
//! the `handy-computer` Hugging Face organization) and the transcribe.cpp
//! runtime (its official Windows bundle with the Vulkan and CPU backends).
//! Every download is pinned to an exact file with its size and SHA-256, so a
//! changed or broken file is never installed.

use serde::{Deserialize, Serialize};

/// transcribe.cpp version whose C API `ffi.rs` mirrors. The runtime must be
/// this exact version (checked through its contract.json).
pub const TRANSCRIBE_VERSION: &str = "0.2.4";
/// Hash of the public C header the runtime was built against (contract.json)
pub const HEADER_HASH: &str = "7df72bf9e667b8c2";

/// Where a speech model runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Vulkan,
    Cpu,
}

impl Backend {
    pub const ALL: [Backend; 2] = [Backend::Vulkan, Backend::Cpu];

    pub fn id(self) -> &'static str {
        match self {
            Backend::Vulkan => "vulkan",
            Backend::Cpu => "cpu",
        }
    }

    pub fn parse(id: &str) -> Option<Backend> {
        Backend::ALL.into_iter().find(|b| b.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            Backend::Vulkan => "Vulkan",
            Backend::Cpu => "CPU",
        }
    }
}

/// The runtime: transcribe.dll with the ggml backend modules, as a tar.gz
/// whose single top-level folder is stripped when unpacking.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeSpec {
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

impl RuntimeSpec {
    pub fn file_name(&self) -> &'static str {
        self.url.rsplit('/').next().unwrap_or(self.url)
    }

    /// Folder name under the runtimes directory
    pub fn dir_name(&self) -> String {
        format!("transcribe-{}-vulkan", TRANSCRIBE_VERSION)
    }
}

/// The official `windows-x86_64-cpu-vulkan` bundle: Vulkan runs on any GPU
/// with a Vulkan driver (NVIDIA, AMD, Intel), the CPU modules everywhere.
pub const RUNTIME: RuntimeSpec = RuntimeSpec {
    url: "https://github.com/handy-computer/transcribe.cpp/releases/download/v0.2.4/transcribe-native-0.2.4-windows-x86_64-cpu-vulkan.tar.gz",
    size: 17_433_342,
    sha256: "09705f54218817c065602ada8fd0f4d13b3f7fbb9d94929eeb3789c6c2b1f34a",
};

/// A downloadable speech model.
#[derive(Debug, Clone, Copy)]
pub struct ModelSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub repo: &'static str,
    pub file: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

impl ModelSpec {
    pub fn url(&self) -> String {
        format!("https://huggingface.co/{}/resolve/main/{}", self.repo, self.file)
    }
}

pub const DEFAULT_MODEL: &str = "parakeet-tdt-0.6b-v3";

/// Q8_0 quantization everywhere: practically the accuracy of the full
/// model (transcribe.cpp's recommended preset) at about half the size.
pub const MODELS: &[ModelSpec] = &[
    ModelSpec {
        id: "parakeet-tdt-0.6b-v3",
        name: "Parakeet TDT 0.6B v3",
        repo: "handy-computer/parakeet-tdt-0.6b-v3-gguf",
        file: "parakeet-tdt-0.6b-v3-Q8_0.gguf",
        size: 739_508_576,
        sha256: "5859f77944efcd8eafa23a6350731960b2b55b2203df51f319665c807d802cc7",
    },
    ModelSpec {
        id: "whisper-large-v3",
        name: "Whisper Large v3",
        repo: "handy-computer/whisper-large-v3-gguf",
        file: "whisper-large-v3-Q8_0.gguf",
        size: 1_668_741_440,
        sha256: "2fa1a5f179f8a511a53e2108db270aa4af3ce08cd976af4180e2854666bb4ba3",
    },
    ModelSpec {
        id: "whisper-large-v3-turbo",
        name: "Whisper Large v3 Turbo",
        repo: "handy-computer/whisper-large-v3-turbo-gguf",
        file: "whisper-large-v3-turbo-Q8_0.gguf",
        size: 886_381_760,
        sha256: "b2e30cc286bc9f3aba4db9099fc7403543497c05ce7100d0d83091ddfd25a183",
    },
    ModelSpec {
        id: "qwen3-asr-0.6b",
        name: "Qwen3 ASR 0.6B",
        repo: "handy-computer/Qwen3-ASR-0.6B-gguf",
        file: "Qwen3-ASR-0.6B-Q8_0.gguf",
        size: 850_423_456,
        sha256: "f081b2d5e23bd669d92cc331d722a8a0681943b8e6f34b48996fd5c319b5acd8",
    },
];

pub fn model(id: &str) -> Option<&'static ModelSpec> {
    MODELS.iter().find(|m| m.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_model_is_in_catalog() {
        assert!(model(DEFAULT_MODEL).is_some());
    }

    #[test]
    fn test_model_url() {
        assert_eq!(
            model("qwen3-asr-0.6b").unwrap().url(),
            "https://huggingface.co/handy-computer/Qwen3-ASR-0.6B-gguf/resolve/main/Qwen3-ASR-0.6B-Q8_0.gguf"
        );
    }

    #[test]
    fn test_backend_ids() {
        for backend in Backend::ALL {
            assert_eq!(Backend::parse(backend.id()), Some(backend));
        }
        assert_eq!(Backend::parse("cuda"), None);
    }

    #[test]
    fn test_checksums_are_sha256() {
        for sha in MODELS.iter().map(|m| m.sha256).chain([RUNTIME.sha256]) {
            assert_eq!(sha.len(), 64);
            assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn test_runtime_names() {
        assert_eq!(
            RUNTIME.file_name(),
            "transcribe-native-0.2.4-windows-x86_64-cpu-vulkan.tar.gz"
        );
        assert_eq!(RUNTIME.dir_name(), "transcribe-0.2.4-vulkan");
    }
}
