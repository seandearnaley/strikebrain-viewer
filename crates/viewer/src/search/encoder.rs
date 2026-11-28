//! The encoder only accepts local files. Its output is normalized by the
//! vendor at the requested Matryoshka width.
use std::io::{BufReader, Read as _};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context as _, Result, ensure};
use image::{DynamicImage, ImageDecoder as _, ImageReader};
use lattice_model_embeddinggemma2::EmbeddingGemma2Model;
use lattice_model_embeddinggemma2::image::EmbeddingGemma2Rgb8Image;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use lattice_model_embeddinggemma2::metal::MetalEmbeddingGemma2State;
use lattice_model_embeddinggemma2::model::{EmbeddingGemma2Part, EmbeddingGemma2Task};

use crate::stage::load::{IMAGE_LIMIT, TEXT_LIMIT};

pub(super) const WIDTH: usize = 256;

pub(super) struct Encoder {
    model: EmbeddingGemma2Model,
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    metal: MetalEmbeddingGemma2State,
}

impl Encoder {
    pub(super) fn load(dir: &Path) -> Result<Self> {
        let model = EmbeddingGemma2Model::from_model_dir(dir)?;
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        let metal = MetalEmbeddingGemma2State::new(&model)?;
        Ok(Self {
            model,
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            metal,
        })
    }

    fn text(&mut self, text: &str, task: EmbeddingGemma2Task) -> Result<Vec<f32>> {
        let ids = self.model.tokenize(&task.format(text))?;
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        let vectors = self
            .model
            .encode_ids_at_widths_metal(&mut self.metal, &ids, &[WIDTH])?;
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        let vectors = self.model.encode_ids_at_widths(&ids, &[WIDTH])?;
        vectors.into_iter().next().context("Missing text embedding")
    }

    pub(super) fn query(&mut self, text: &str) -> Result<Vec<f32>> {
        self.text(text, EmbeddingGemma2Task::Query)
    }

    pub(super) fn file(
        &mut self,
        file: std::fs::File,
        picture: bool,
        cancel: &AtomicBool,
    ) -> Result<Vec<f32>> {
        if picture {
            ensure!(
                file.metadata()?.len() <= IMAGE_LIMIT,
                "Image exceeds the viewer's size limit"
            );
            let mut decoder = ImageReader::new(BufReader::new(file))
                .with_guessed_format()?
                .into_decoder()?;
            let orientation = decoder.orientation()?;
            let mut image = DynamicImage::from_decoder(decoder)?;
            image.apply_orientation(orientation);
            let rgb = image.into_rgb8();
            let part = EmbeddingGemma2Part::Image(EmbeddingGemma2Rgb8Image::new(
                rgb.width() as usize,
                rgb.height() as usize,
                rgb.into_raw(),
            )?);
            let mut cancelled = || cancel.load(Ordering::Relaxed);
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            let (encoded, _) = self.model.encode_parts_at_widths_metal(
                &mut self.metal,
                &[part],
                &[WIDTH],
                &mut cancelled,
            )?;
            #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
            let encoded = self
                .model
                .encode_parts_at_widths(&[part], &[WIDTH], &mut cancelled)?;
            return encoded
                .embeddings
                .into_iter()
                .next()
                .context("Missing image embedding");
        }
        ensure!(
            file.metadata()?.len() <= TEXT_LIMIT,
            "Text exceeds the viewer's size limit"
        );
        let mut text = String::new();
        file.take(TEXT_LIMIT + 1).read_to_string(&mut text)?;
        ensure!(
            text.len() as u64 <= TEXT_LIMIT && !text.contains('\0'),
            "Unsupported text file"
        );
        // Bound attention cost, and give every passage a chance to match.
        let mut rest = text.as_str();
        let mut vectors = Vec::new();
        while !rest.is_empty() {
            ensure!(!cancel.load(Ordering::Relaxed), "Paused");
            let (passage, tail) = rest.split_at(rest.floor_char_boundary(rest.len().min(1024)));
            if !passage.trim().is_empty() {
                vectors.extend(self.text(passage, EmbeddingGemma2Task::Document)?);
            }
            rest = tail;
        }
        ensure!(!vectors.is_empty(), "Empty text file");
        Ok(vectors)
    }
}
