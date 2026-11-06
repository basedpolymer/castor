//! Model catalog: every downloadable model, with a pinned URL (commit-hashed Hugging Face
//! `resolve` URL or GitHub release), SHA-256, size and licence.

use castor_core::download::Asset;

/// What a model does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task {
    /// Salient object matte (background removal). Fixed 1024 × 1024 input.
    Matte,
    /// Super-resolution with a native factor.
    Upscale { native: u32 },
    /// Face boxes.
    Faces,
}

#[derive(Clone, Debug)]
pub struct Model {
    /// Stable id (used in options, `Requirement::Model` and the download path).
    pub id: &'static str,
    /// Display name (a proper noun, not translated).
    pub name: &'static str,
    pub task: Task,
    pub asset: Asset,
    /// Where the weights come from, for the licence table in Settings → Models.
    pub source: &'static str,
}

const CATALOG: &[Model] = &[
    Model {
        id: "isnet-general-use",
        name: "ISNet (general use)",
        task: Task::Matte,
        asset: Asset {
            id: "models/isnet-general-use.onnx",
            url: "https://github.com/danielgatis/rembg/releases/download/v0.0.0/isnet-general-use.onnx",
            sha256: "60920e99c45464f2ba57bee2ad08c919a52bbf852739e96947fbb4358c0d964a",
            size: 178_648_008,
            license: "Apache-2.0",
        },
        source: "https://github.com/xuebinqin/DIS (ONNX export by the rembg project)",
    },
    Model {
        id: "birefnet-lite",
        name: "BiRefNet lite",
        task: Task::Matte,
        asset: Asset {
            id: "models/birefnet-lite.onnx",
            url: "https://huggingface.co/onnx-community/BiRefNet_lite-ONNX/resolve/de15b22ba131738a16dff04aab8bdf8dc32e3ac1/onnx/model.onnx",
            sha256: "5600024376f572a557870a5eb0afb1e5961636bef4e1e22132025467d0f03333",
            size: 224_005_088,
            license: "MIT",
        },
        source: "https://github.com/ZhengPeng7/BiRefNet (ONNX export by onnx-community)",
    },
    Model {
        id: "realesrgan-x4plus",
        name: "Real-ESRGAN x4plus",
        task: Task::Upscale { native: 4 },
        asset: Asset {
            id: "models/realesrgan-x4plus.onnx",
            url: "https://huggingface.co/SceneWorks/real-esrgan-onnx/resolve/09f741bac80a246b407da3ee902bf5f3291b602f/real_esrgan_x4.onnx",
            sha256: "5c586662929cbc686c1a5c38d9c060dbdb4ea5863a1f7672b8c0761e6b89c033",
            size: 67_051_616,
            license: "BSD-3-Clause",
        },
        source: "https://github.com/xinntao/Real-ESRGAN (RealESRGAN_x4plus.pth, ONNX export by SceneWorks)",
    },
    Model {
        id: "realesrgan-x2plus",
        name: "Real-ESRGAN x2plus",
        task: Task::Upscale { native: 2 },
        asset: Asset {
            id: "models/realesrgan-x2plus.onnx",
            url: "https://huggingface.co/SceneWorks/real-esrgan-onnx/resolve/09f741bac80a246b407da3ee902bf5f3291b602f/real_esrgan_x2.onnx",
            sha256: "7115ba92e8a1bfa63d68558ef006ef3d91273a068d321b1439f8bb1c9179002c",
            size: 67_073_434,
            license: "BSD-3-Clause",
        },
        source: "https://github.com/xinntao/Real-ESRGAN (RealESRGAN_x2plus.pth, ONNX export by SceneWorks)",
    },
    Model {
        id: "realesrgan-x4plus-anime6b",
        name: "Real-ESRGAN x4plus anime 6B",
        task: Task::Upscale { native: 4 },
        asset: Asset {
            id: "models/realesrgan-x4plus-anime6b.onnx",
            url: "https://huggingface.co/RekluzLabs/realesrgan_anime6b.onnx/resolve/d5377ec6204295bca15f0a7967579dc5e25e2ae6/realesrgan_anime6b.onnx",
            sha256: "45bd54934aeabe8df744c8fdacb9e8846c9b55cb4e60c499db77405d1625a667",
            size: 18_352_469,
            license: "BSD-3-Clause",
        },
        source: "https://github.com/xinntao/Real-ESRGAN (RealESRGAN_x4plus_anime_6B.pth, ONNX export by RekluzLabs)",
    },
    Model {
        id: "realesr-general-x4v3",
        name: "Real-ESRGAN general v3 (fast)",
        task: Task::Upscale { native: 4 },
        asset: Asset {
            id: "models/realesr-general-x4v3.onnx",
            url: "https://huggingface.co/CoderViking/realesr-general-x4v3-onnx/resolve/c6a971706797c7502945a2b4c4274fce4900d4ab/realesr-general-x4v3.onnx",
            sha256: "1940a93ee08283a0a7286183186357b1688fe9fa8ede74604b424586aaddf112",
            size: 4_866_417,
            license: "BSD-3-Clause",
        },
        source: "https://github.com/xinntao/Real-ESRGAN (realesr-general-x4v3.pth, ONNX export by CoderViking)",
    },
    Model {
        id: "yunet",
        name: "YuNet face detector",
        task: Task::Faces,
        asset: Asset {
            id: "models/yunet-2023mar.onnx",
            url: "https://huggingface.co/opencv/face_detection_yunet/resolve/3cc26e7f1014a5ee5d74a42acee58bafc9d0a310/face_detection_yunet_2023mar.onnx",
            sha256: "8f2383e4dd3cfbb4553ea8718107fc0423210dc964f9f4280604804ed2552fa4",
            size: 232_589,
            license: "MIT",
        },
        source: "https://github.com/opencv/opencv_zoo/tree/main/models/face_detection_yunet",
    },
];

/// Catalog: id, display name, asset (url, sha256, size, licence).
pub fn catalog() -> &'static [Model] {
    CATALOG
}

pub fn get(id: &str) -> Option<&'static Model> {
    CATALOG.iter().find(|m| m.id == id)
}

/// The download asset of a model, by id.
pub fn asset(id: &str) -> Option<castor_core::download::Asset> {
    get(id).map(|m| m.asset.clone())
}

#[cfg(test)]
mod tests {
    #[test]
    fn catalog_is_sane() {
        for m in super::catalog() {
            assert_eq!(m.asset.sha256.len(), 64, "{}", m.id);
            assert!(m.asset.size > 0);
            assert!(m.asset.url.starts_with("https://"));
            assert!(m.asset.id.starts_with("models/"));
        }
    }
}
