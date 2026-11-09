//! Sources/StreamProtocol/MacMenu.swift: kinds 24 (host to device), 25 and 27 (device to host).
//! Every field optional; ids are paths of child indexes ("2.9") valid within one `version`.

use serde::{Deserialize, Serialize};

/// `MacMenu` (MacMenu.swift:33-67), kind 24: the top level (`menus`) or the answer to one fetch
/// (`menu`, `items`) or press (`pressed`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacMenu {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "bundleID")]
    pub bundle_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menus: Option<Vec<MacMenuItem>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answering: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<MacMenuItem>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub more: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pressed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// `MacMenuItem` (MacMenu.swift:69-93). `key` is the shortcut as text ("⇧⌘S").
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacMenuItem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub separator: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mark: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submenu: Option<bool>,
}

/// `FetchMenu` (MacMenu.swift:95-110), kind 27: without `id` it is the subscription.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchMenu {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<i64>,
}

/// `PressMenuItem` (MacMenu.swift:112-124), kind 25.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PressMenuItem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<i64>,
}
