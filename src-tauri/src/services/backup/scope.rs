//! バックアップの区分(設定値・ギャラリー・プリセット)と、それぞれが扱うテーブル(docs/35 §1)。
//!
//! 取得・復元は「対象テーブルの全行」を直接操作せず、必ずこの定義を経由する。
//! (ユーザー機能の導入時に、ここへ `user_id` の絞り込みを足せるようにするため。docs/35 §8.5)

use serde::{Deserialize, Serialize};

/// API キーの設定キー。バックアップには既定で含めない(docs/35 §2)。
pub const API_KEY_KEYS: [&str; 2] = ["assistant.api_key", "assistant.key_verified"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Include {
    pub settings: bool,
    pub gallery: bool,
    pub presets: bool,
}

impl Include {
    pub fn any(&self) -> bool {
        self.settings || self.gallery || self.presets
    }

    /// `self` の区分がすべて `other` に含まれる。
    pub fn is_subset_of(&self, other: &Include) -> bool {
        (!self.settings || other.settings)
            && (!self.gallery || other.gallery)
            && (!self.presets || other.presets)
    }

    pub fn sections(&self) -> Vec<Section> {
        let mut v = Vec::new();
        if self.settings {
            v.push(Section::Settings);
        }
        if self.gallery {
            v.push(Section::Gallery);
        }
        if self.presets {
            v.push(Section::Presets);
        }
        v
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Settings,
    Gallery,
    Presets,
}

/// 区分ごとのテーブル(**挿入順 = 親 → 子**。削除は逆順)。
pub fn tables(section: Section) -> &'static [&'static str] {
    match section {
        Section::Settings => &["settings", "assistant_conversations", "assistant_messages"],
        Section::Gallery => &["jobs", "images"],
        Section::Presets => &["presets"],
    }
}

/// 区分に含まれない(OFF の)区分の一覧。
pub fn excluded_sections(include: &Include) -> Vec<Section> {
    [Section::Settings, Section::Gallery, Section::Presets]
        .into_iter()
        .filter(|s| !include.sections().contains(s))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subset_and_any() {
        let all = Include {
            settings: true,
            gallery: true,
            presets: true,
        };
        let none = Include {
            settings: false,
            gallery: false,
            presets: false,
        };
        assert!(all.any());
        assert!(!none.any());
        assert!(none.is_subset_of(&all));
        assert!(!all.is_subset_of(&none));
    }

    #[test]
    fn excluded_sections_are_the_complement() {
        let inc = Include {
            settings: true,
            gallery: false,
            presets: false,
        };
        assert_eq!(
            excluded_sections(&inc),
            vec![Section::Gallery, Section::Presets]
        );
    }

    #[test]
    fn images_come_after_jobs_so_children_follow_parents() {
        assert_eq!(tables(Section::Gallery), ["jobs", "images"]);
        let t = tables(Section::Settings);
        let c = t.iter().position(|x| *x == "assistant_conversations");
        let m = t.iter().position(|x| *x == "assistant_messages");
        assert!(c < m);
    }
}
