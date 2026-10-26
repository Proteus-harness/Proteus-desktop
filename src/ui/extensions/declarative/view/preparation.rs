//! Native controls depend on accepted process data; SVGs also depend on DPI.
//! Local redraws keep both, including edits, search and disclosure state.

#[derive(Default)]
pub(super) struct Preparation {
    fields: Option<u64>,
    images: Option<(u64, u32)>,
}

pub(super) struct Changed {
    pub fields: bool,
    pub images: bool,
}

impl Preparation {
    pub(super) fn for_frame(&mut self, revision: u64, scale: f32) -> Changed {
        let images = (revision, scale.to_bits());
        let changed = Changed {
            fields: self.fields != Some(revision),
            images: self.images != Some(images),
        };
        self.fields = Some(revision);
        self.images = Some(images);
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_redraws_reuse_controls_and_rasters_until_new_process_data() {
        let mut preparation = Preparation::default();
        let mut passes = (0, 0);
        for revision in [1; 120].into_iter().chain([2; 120]) {
            let changed = preparation.for_frame(revision, 1.);
            passes.0 += usize::from(changed.fields);
            passes.1 += usize::from(changed.images);
        }
        assert_eq!(passes, (2, 2));
    }

    #[test]
    fn zoom_or_monitor_changes_rerasterize_without_repreparing_controls() {
        let mut preparation = Preparation::default();
        preparation.for_frame(7, 1.);
        for scale in [1.25, 2., 1.] {
            let changed = preparation.for_frame(7, scale);
            assert!(!changed.fields);
            assert!(changed.images);
            let repeated = preparation.for_frame(7, scale);
            assert!(!repeated.fields && !repeated.images);
        }
        let updated = preparation.for_frame(8, 1.);
        assert!(updated.fields && updated.images);
    }

    #[test]
    fn restarted_or_wrapped_tree_prepares_again() {
        let mut preparation = Preparation::default();
        preparation.for_frame(u64::MAX, 1.);
        let wrapped = preparation.for_frame(0, 1.);
        assert!(wrapped.fields && wrapped.images);
        preparation = Preparation::default();
        let restarted = preparation.for_frame(0, 1.);
        assert!(restarted.fields && restarted.images);
    }
}
