//! Images of the composer draft: picked or dropped files and clipboard
//! pastes, checked against the host limits before they are sent.

use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::*;
use gpui_kit::*;
use proteus_contracts::domain::{ImageAttachment, MAX_IMAGE_BYTES, MAX_INPUT_IMAGES};

use super::Chat;
use super::attachment_io::{self, Batch, Budget, Prepared, Source};
use crate::ui::kit::Hint as _;
use crate::ui::tip::Tip as _;
use crate::ui::{kit, prefs};

const THUMB: f32 = 56.;

/// An image of the draft with the bytes its preview decodes.
pub(super) struct Attachment {
    image: ImageAttachment,
    preview: Arc<Image>,
}

#[derive(Default)]
pub(super) struct Attachments {
    items: Vec<Attachment>,
}

impl Attachments {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn images(&self) -> Vec<ImageAttachment> {
        self.items.iter().map(|item| item.image.clone()).collect()
    }

    fn budget(&self) -> Budget {
        Budget {
            slots: MAX_INPUT_IMAGES.saturating_sub(self.items.len()),
            bytes: MAX_IMAGE_BYTES
                .saturating_sub(self.items.iter().map(|item| item.preview.bytes.len()).sum()),
        }
    }

    fn remove(&mut self, index: usize) {
        if index < self.items.len() {
            self.items.remove(index);
        }
    }

    /// Adds an image the host accepts; the error says why it would not.
    #[cfg(test)]
    fn add(&mut self, name: String, bytes: Vec<u8>) -> Result<(), String> {
        self.add_prepared(attachment_io::prepare(name, bytes)?)
    }

    fn add_prepared(&mut self, prepared: Prepared) -> Result<(), String> {
        let Prepared { image, bytes } = prepared;
        let name = &image.name;
        let format = ImageFormat::from_mime_type(&image.mime_type)
            .ok_or_else(|| format!("{name}: поддерживаются PNG, JPEG, WebP и GIF"))?;
        if self.items.len() >= MAX_INPUT_IMAGES {
            return Err(format!(
                "{name}: к сообщению можно приложить не больше {MAX_INPUT_IMAGES} изображений"
            ));
        }
        let total: usize = self.items.iter().map(|item| item.preview.bytes.len()).sum();
        if total + bytes.len() > MAX_IMAGE_BYTES {
            return Err(format!("{name}: изображения сообщения вместе больше 5 МиБ"));
        }
        self.items.push(Attachment {
            image,
            preview: Arc::new(Image::from_bytes(format, bytes)),
        });
        Ok(())
    }
}

impl Chat {
    pub(super) fn attach_files(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        self.load_attachments(
            paths.iter().cloned().map(Source::File).collect(),
            vec![],
            cx,
        );
    }

    /// Takes images and copied files from a paste; text, even beside an
    /// image, stays for the input.
    pub(super) fn attach_clipboard(
        &mut self,
        item: &ClipboardItem,
        cx: &mut Context<Self>,
    ) -> bool {
        if item.text().is_some_and(|text| !text.is_empty()) {
            return false;
        }
        if self.attachments_loading {
            self.show_notice("Дождитесь загрузки изображений", cx);
            return true;
        }
        let mut sources = Vec::new();
        let mut errors = Vec::new();
        // Bound clipboard byte clones too. File paths themselves are cheap;
        // the background reader charges them against the same final budget.
        let mut copied = self.attachments.budget();
        for entry in item.entries() {
            match entry {
                ClipboardEntry::Image(image) => {
                    let name = format!("clipboard.{}", image.format.extension());
                    if copied.slots == 0 || image.bytes.len() > copied.bytes {
                        if errors.is_empty() {
                            errors.push("Изображения из буфера не помещаются в лимит сообщения (4 изображения, 5 МиБ)".into());
                        }
                        continue;
                    }
                    copied.slots -= 1;
                    copied.bytes -= image.bytes.len();
                    sources.push(Source::Bytes(name, image.bytes.clone()));
                }
                ClipboardEntry::ExternalPaths(paths) => {
                    sources.extend(paths.paths().iter().cloned().map(Source::File));
                }
                ClipboardEntry::String(_) => {}
            }
        }
        if sources.is_empty() && errors.is_empty() {
            return false;
        }
        self.load_attachments(sources, errors, cx);
        true
    }

    pub(super) fn choose_images(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.attachments_loading {
            return;
        }
        let generation = self.attachment_generation;
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Прикрепить".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            this.update(cx, |this, cx| {
                if this.attachment_generation == generation {
                    this.attach_files(&paths, cx);
                } else {
                    this.show_notice("Черновик изменился; прикрепите изображения ещё раз", cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn load_attachments(
        &mut self,
        sources: Vec<Source>,
        errors: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if self.attachments_loading {
            self.show_notice("Дождитесь загрузки изображений", cx);
            return;
        }
        if sources.is_empty() {
            if !errors.is_empty() {
                self.show_notice(errors.join("\n"), cx);
            }
            return;
        }
        let budget = self.attachments.budget();
        let generation = self.attachment_generation;
        self.attachments_loading = true;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let mut batch = cx
                .background_executor()
                .spawn(async move { attachment_io::read_batch(sources, budget) })
                .await;
            batch.errors.extend(errors);
            this.update(cx, |this, cx| {
                this.attachments_loading = false;
                if generation == this.attachment_generation {
                    this.finish_attachments(batch, cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn finish_attachments(&mut self, batch: Batch, cx: &mut Context<Self>) {
        let mut errors = batch.errors;
        for image in batch.images {
            if let Err(error) = self.attachments.add_prepared(image) {
                errors.push(error);
            }
        }
        if !errors.is_empty() {
            self.show_notice(errors.join("\n"), cx);
        }
    }

    pub(super) fn render_attachments(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if self.attachments.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let thumbs = self
            .attachments
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                div()
                    .id(("attachment", index))
                    .group("attachment")
                    .relative()
                    .size(prefs::scaled(THUMB))
                    .rounded(theme.radius)
                    .border_1()
                    .border_color(theme.border)
                    .tip_text(item.image.name.clone())
                    .child(
                        img(item.preview.clone())
                            .size_full()
                            .rounded(theme.radius)
                            .object_fit(ObjectFit::Cover),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0p5()
                            .right_0p5()
                            .invisible()
                            .group_hover("attachment", |style| style.visible())
                            .child(
                                kit::button(("attachment-remove", index))
                                    .icon(IconName::Close)
                                    .hint("Убрать", cx)
                                    .xsmall()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.attachments.remove(index);
                                        cx.notify();
                                    })),
                            ),
                    )
            });
        Some(
            h_flex()
                .flex_wrap()
                .gap_2()
                .px_1()
                .pt_1()
                .children(thumbs)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use proteus_contracts::domain::{MAX_IMAGE_BYTES, MAX_INPUT_IMAGES};

    use super::Attachments;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n";

    fn png(size: usize) -> Vec<u8> {
        let mut bytes = PNG.to_vec();
        bytes.resize(size.max(PNG.len()), 0);
        bytes
    }

    #[test]
    fn draft_takes_what_the_host_accepts() {
        let mut draft = Attachments::default();
        assert!(draft.add("notes.txt".into(), b"plain".to_vec()).is_err());
        for index in 0..MAX_INPUT_IMAGES {
            draft.add(format!("{index}.png"), png(16)).unwrap();
        }
        assert!(draft.add("extra.png".into(), png(16)).is_err());

        draft.remove(0);
        assert!(draft.add("big.png".into(), png(MAX_IMAGE_BYTES)).is_err());
        draft
            .add("fits.png".into(), png(MAX_IMAGE_BYTES - 48))
            .unwrap();

        let images = draft.images();
        assert_eq!(images.len(), MAX_INPUT_IMAGES);
        assert_eq!(images[0].mime_type, "image/png");
    }
}
