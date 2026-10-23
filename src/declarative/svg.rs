//! SVG is source data, never a path or browser document. GPUI's usvg image
//! resolver otherwise opens local `href` files while parsing an image.

use anyhow::{Result, bail};

pub fn validate(source: &str) -> Result<()> {
    if source.len() > 1024 * 1024 {
        bail!("svg source exceeds 1 MiB");
    }
    let document = roxmltree::Document::parse(source)?;
    let root = document.root_element();
    if root.tag_name().name() != "svg" {
        bail!("svg source needs an svg root element");
    }
    for node in root.descendants().filter(|node| node.is_element()) {
        if matches!(node.tag_name().name(), "foreignObject" | "script") {
            bail!("svg supports native drawing elements, not HTML or scripts");
        }
        for attribute in node.attributes() {
            if attribute.name() == "href" && !attribute.value().trim_start().starts_with('#') {
                bail!("svg references must be local #fragments; external assets are not loaded");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_local_paint_references_but_never_opens_href_files() {
        validate(r##"<svg xmlns="http://www.w3.org/2000/svg"><defs><path id="p" d="M0 0"/></defs><use href="#p"/></svg>"##).unwrap();
        for source in [
            r#"<svg><image href="/tmp/private.png"/></svg>"#,
            r#"<svg xmlns:xlink="http://www.w3.org/1999/xlink"><image xlink:href="https://example.com/p.png"/></svg>"#,
            r#"<svg><image href="&#47;tmp/private.png"/></svg>"#,
            "<svg><foreignObject/></svg>",
            "<svg",
        ] {
            assert!(validate(source).is_err(), "{source}");
        }
    }
}
