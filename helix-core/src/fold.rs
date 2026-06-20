use std::borrow::Cow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FoldKind {
    Block,
    Comment,
    Imports,
    Region,
    Function,
    Method,
    Class,
    Type,
}

impl FoldKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Block => "block",
            Self::Comment => "comment",
            Self::Imports => "imports",
            Self::Region => "region",
            Self::Function => "function",
            Self::Method => "method",
            Self::Class => "class",
            Self::Type => "type",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldSource {
    TreeSitter,
    Lsp,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoldRange {
    pub start_line: usize,
    pub end_line: usize,
    pub start_char: usize,
    pub end_char: usize,
    pub kind: FoldKind,
    pub source: FoldSource,
    pub placeholder: Cow<'static, str>,
}

impl FoldRange {
    pub fn new(
        start_line: usize,
        end_line: usize,
        start_char: usize,
        end_char: usize,
        placeholder: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self {
            start_line,
            end_line,
            start_char,
            end_char,
            kind: FoldKind::Block,
            source: FoldSource::TreeSitter,
            placeholder: placeholder.into(),
        }
    }

    pub fn with_kind(mut self, kind: FoldKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn with_source(mut self, source: FoldSource) -> Self {
        self.source = source;
        self
    }

    pub fn is_valid(&self) -> bool {
        self.start_line < self.end_line && self.start_char < self.end_char
    }

    pub fn contains_line(&self, line: usize) -> bool {
        self.start_line <= line && line <= self.end_line
    }
}

pub fn normalize_folds(folds: &mut Vec<FoldRange>) {
    folds.retain(FoldRange::is_valid);
    folds.sort_unstable_by_key(|fold| (fold.start_char, fold.end_char));

    let mut normalized: Vec<FoldRange> = Vec::with_capacity(folds.len());

    for fold in folds.drain(..) {
        let Some(last) = normalized.last_mut() else {
            normalized.push(fold);
            continue;
        };

        if fold.start_char > last.end_char {
            normalized.push(fold);
            continue;
        }

        if fold.end_char > last.end_char {
            last.end_char = fold.end_char;
            last.end_line = fold.end_line;
        } else if fold.end_char == last.end_char {
            last.end_line = last.end_line.max(fold.end_line);
        }

        if last.kind != fold.kind {
            last.kind = FoldKind::Block;
        }
        if last.source != fold.source {
            last.source = FoldSource::Manual;
        }

        let hidden_lines = last.end_line.saturating_sub(last.start_line);
        last.placeholder = format!(" ⋯ {hidden_lines} lines").into();
    }

    *folds = normalized;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fold(start_line: usize, end_line: usize, start_char: usize, end_char: usize) -> FoldRange {
        FoldRange::new(
            start_line,
            end_line,
            start_char,
            end_char,
            format!(" ⋯ {} lines", end_line.saturating_sub(start_line)),
        )
    }

    #[test]
    fn valid_folds_span_multiple_lines_and_forward_chars() {
        assert!(FoldRange::new(1, 3, 10, 30, " ⋯ 2 lines").is_valid());
        assert!(!FoldRange::new(1, 1, 10, 30, " ⋯ 0 lines").is_valid());
        assert!(!FoldRange::new(1, 3, 30, 10, " ⋯ 2 lines").is_valid());
    }

    #[test]
    fn normalize_keeps_valid_non_overlapping_folds_separate() {
        let mut folds = vec![fold(0, 2, 10, 30), fold(4, 6, 50, 70)];

        normalize_folds(&mut folds);

        assert_eq!(folds.len(), 2);
        assert_eq!((folds[0].start_char, folds[0].end_char), (10, 30));
        assert_eq!((folds[1].start_char, folds[1].end_char), (50, 70));
    }

    #[test]
    fn normalize_merges_exact_duplicates() {
        let mut folds = vec![fold(0, 2, 10, 30), fold(0, 2, 10, 30)];

        normalize_folds(&mut folds);

        assert_eq!(folds.len(), 1);
        assert_eq!((folds[0].start_line, folds[0].end_line), (0, 2));
        assert_eq!((folds[0].start_char, folds[0].end_char), (10, 30));
    }

    #[test]
    fn normalize_merges_same_start_to_largest_end() {
        let mut folds = vec![fold(0, 2, 10, 30), fold(0, 4, 10, 50)];

        normalize_folds(&mut folds);

        assert_eq!(folds.len(), 1);
        assert_eq!((folds[0].start_line, folds[0].end_line), (0, 4));
        assert_eq!((folds[0].start_char, folds[0].end_char), (10, 50));
    }

    #[test]
    fn normalize_merges_nested_active_folds() {
        let mut folds = vec![fold(0, 5, 10, 80), fold(2, 3, 35, 55)];

        normalize_folds(&mut folds);

        assert_eq!(folds.len(), 1);
        assert_eq!((folds[0].start_line, folds[0].end_line), (0, 5));
        assert_eq!((folds[0].start_char, folds[0].end_char), (10, 80));
    }

    #[test]
    fn normalize_merges_touching_char_ranges() {
        let mut folds = vec![fold(0, 2, 10, 30), fold(2, 4, 30, 60)];

        normalize_folds(&mut folds);

        assert_eq!(folds.len(), 1);
        assert_eq!((folds[0].start_line, folds[0].end_line), (0, 4));
        assert_eq!((folds[0].start_char, folds[0].end_char), (10, 60));
    }

    #[test]
    fn normalize_recomputes_placeholder_for_merged_span() {
        let mut folds = vec![
            FoldRange::new(0, 2, 10, 30, " custom"),
            FoldRange::new(2, 5, 30, 90, " other"),
        ];

        normalize_folds(&mut folds);

        assert_eq!(folds.len(), 1);
        assert_eq!(folds[0].placeholder.as_ref(), " ⋯ 5 lines");
    }

    #[test]
    fn normalize_demotes_mixed_kinds_to_block() {
        let mut folds = vec![
            fold(0, 2, 10, 30).with_kind(FoldKind::Comment),
            fold(2, 4, 30, 60).with_kind(FoldKind::Function),
        ];

        normalize_folds(&mut folds);

        assert_eq!(folds.len(), 1);
        assert_eq!(folds[0].kind, FoldKind::Block);
    }

    #[test]
    fn normalize_demotes_mixed_sources_to_manual() {
        let mut folds = vec![
            fold(0, 2, 10, 30).with_source(FoldSource::TreeSitter),
            fold(2, 4, 30, 60).with_source(FoldSource::Lsp),
        ];

        normalize_folds(&mut folds);

        assert_eq!(folds.len(), 1);
        assert_eq!(folds[0].source, FoldSource::Manual);
    }
}
