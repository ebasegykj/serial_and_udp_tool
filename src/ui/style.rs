//! 方案 B 样式系统：所有 UI 数值集中在 `ui_style.json`，
//! 支持运行时 F12 调试面板实时拖动微调 + 保存。

use egui::{Color32, FontId, TextStyle};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// UI 样式配置。exe 同目录 `ui_style.json`，无文件时用默认值。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StyleConfig {
    // —— 布局 ——
    pub left_panel_width: f32,
    pub row_spacing: f32,
    pub plot_min_height: f32,
    // —— 字体 ——
    pub font_size_body: f32,
    pub font_size_small: f32,
    pub font_size_heading: f32,
    pub font_size_monospace: f32,
    // —— 绘图 ——
    pub line_width: f32,
    pub marker_size: f32,
    pub axis_tip_width: f32,
    // —— 颜色（#RRGGBB）——
    pub accent: String,
    pub bg_plot: String,
    pub text_normal: String,
    pub text_dim: String,
    // —— 主题（🌙 循环切换，0=深色）——
    #[serde(default)]
    pub theme_index: usize,
}

/// 主题配色（对齐 Python 15 版 THEME_LIST 顺序与色值）。
pub struct ThemePalette {
    pub name: &'static str,
    pub bg: &'static str,
    pub panel: &'static str,
    pub panel_2: &'static str,
    pub border: &'static str,
    pub text: &'static str,
    pub text_dim: &'static str,
    pub accent: &'static str,
    pub plot_bg: &'static str,
    pub hover: &'static str,
    pub active: &'static str,
}

pub const THEMES: [ThemePalette; 8] = [
    ThemePalette { name: "深色", bg: "#13141c", panel: "#252938", panel_2: "#2d3244", border: "#3a4056", text: "#ffffff", text_dim: "#9da3bb", accent: "#8b9bff", plot_bg: "#0e0f16", hover: "#39405a", active: "#4a5273" },
    ThemePalette { name: "原始", bg: "#d9d9d9", panel: "#f0f0f0", panel_2: "#ffffff", border: "#a0a0a0", text: "#000000", text_dim: "#606060", accent: "#0078d4", plot_bg: "#ffffff", hover: "#d0e3f5", active: "#b9d4ee" },
    ThemePalette { name: "淡绿", bg: "#f0f7f2", panel: "#ffffff", panel_2: "#e3f0e7", border: "#b8d8c1", text: "#1a2e1f", text_dim: "#5a7a63", accent: "#2d8a4e", plot_bg: "#fafdfb", hover: "#d5ebdb", active: "#bfe0c9" },
    ThemePalette { name: "淡蓝", bg: "#eef4fb", panel: "#ffffff", panel_2: "#dce8f5", border: "#aec8e0", text: "#1a2233", text_dim: "#5a6b85", accent: "#2c6bb5", plot_bg: "#fafcff", hover: "#d4e4f5", active: "#bed4ec" },
    ThemePalette { name: "米黄", bg: "#faf6ec", panel: "#ffffff", panel_2: "#f2ecdb", border: "#d4c9a8", text: "#2e2416", text_dim: "#7a6a4a", accent: "#b8860b", plot_bg: "#fffdf7", hover: "#ecdfc0", active: "#dfcfa6" },
    ThemePalette { name: "粉色", bg: "#fdf2f8", panel: "#ffffff", panel_2: "#fce7f3", border: "#f9a8d4", text: "#4a1d3a", text_dim: "#9d5c82", accent: "#db2777", plot_bg: "#fff7fb", hover: "#f9cfdf", active: "#f4b7cd" },
    ThemePalette { name: "Mac", bg: "#e0e0e6", panel: "#f5f5f7", panel_2: "#ffffff", border: "#c8c8d0", text: "#1d1d1f", text_dim: "#6e6e73", accent: "#007aff", plot_bg: "#fbfbfd", hover: "#d5d5dd", active: "#c4c4cd" },
    ThemePalette { name: "Win", bg: "#e8e8ed", panel: "#f7f7fa", panel_2: "#ffffff", border: "#d0d0d8", text: "#1a1a1f", text_dim: "#6e6e78", accent: "#0067c0", plot_bg: "#fbfbfd", hover: "#dbe4ee", active: "#c8d8ea" },
];


impl Default for StyleConfig {
    fn default() -> Self {
        Self {
            left_panel_width: 300.0,
            row_spacing: 6.0,
            plot_min_height: 380.0,
            font_size_body: 14.0,
            font_size_small: 12.0,
            font_size_heading: 17.0,
            font_size_monospace: 13.0,
            line_width: 1.5,
            marker_size: 2.0,
            axis_tip_width: 25.0,
            accent: "#8b9bff".into(),
            bg_plot: "#0e0f16".into(),
            text_normal: "#C8CAD4".into(),
            text_dim: "#8A8FA3".into(),
            theme_index: 0,
        }
    }
}

fn hex_to_color(s: &str) -> Color32 {
    match Color32::from_hex(s) {
        Ok(c) => c,
        Err(_) => Color32::LIGHT_GRAY,
    }
}

impl StyleConfig {
    pub fn config_path() -> PathBuf {
        // 与 .config.json 同目录策略：exe 同目录（或当前目录兜底）
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("ui_style.json")))
            .unwrap_or_else(|| PathBuf::from("ui_style.json"))
    }

    pub fn load() -> Self {
        let path = Self::config_path();
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path();
        let data = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, data).map_err(|e| e.to_string())
    }

    pub fn accent_color(&self) -> Color32 {
        hex_to_color(&self.accent)
    }
    pub fn bg_plot_color(&self) -> Color32 {
        hex_to_color(&self.bg_plot)
    }
    pub fn text_normal(&self) -> Color32 {
        hex_to_color(&self.text_normal)
    }
    pub fn text_dim(&self) -> Color32 {
        hex_to_color(&self.text_dim)
    }

    /// 当前主题（theme_index 越界时回退深色）。
    pub fn theme(&self) -> &'static ThemePalette {
        &THEMES[self.theme_index.min(THEMES.len() - 1)]
    }

    /// 🌙 循环切换到下一个主题，并把配色写入颜色字段（立即生效，可保存）。
    pub fn cycle_theme(&mut self) {
        self.theme_index = (self.theme_index + 1) % THEMES.len();
        self.apply_theme_palette();
    }

    /// 把当前主题色写入颜色字段。
    pub fn apply_theme_palette(&mut self) {
        let t = self.theme();
        self.accent = t.accent.into();
        self.bg_plot = t.plot_bg.into();
        self.text_normal = t.text.into();
        self.text_dim = t.text_dim.into();
    }

    /// 应用主题到 egui Visuals（配色全部来自 THEMES[theme_index]）。
    pub fn apply_theme(&self, ctx: &egui::Context) {
        let t = self.theme();
        let hex = |s: &str| Color32::from_hex(s).unwrap_or(Color32::GRAY);
        let panel = hex(t.panel);
        let bg = hex(t.bg);
        let panel2 = hex(t.panel_2);
        let border = hex(t.border);
        let text = hex(t.text);
        let dim = hex(t.text_dim);
        // 深色主题走 dark 基线，浅色主题走 light 基线（控件按压/hover 观感一致）
        let mut v = if self.theme_index == 0 {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        v.window_fill = panel; // CentralPanel（右侧卡片区）
        v.panel_fill = bg; // SidePanel（左栏底 = 窗口底色，卡片浮在上面）
        v.extreme_bg_color = panel2; // 输入框 / 图例底色
        v.window_stroke = egui::Stroke::new(1.0_f32, border);
        v.selection.bg_fill = self.accent_color();
        v.selection.stroke.color = Color32::WHITE;
        v.hyperlink_color = self.accent_color();
        for w in [
            &mut v.widgets.noninteractive,
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.bg_stroke.color = border;
            w.fg_stroke.color = text;
        }
        v.widgets.noninteractive.fg_stroke.color = dim;
        v.widgets.noninteractive.bg_fill = panel;
        v.widgets.noninteractive.weak_bg_fill = panel;
        v.widgets.open.bg_fill = panel;
        v.widgets.open.weak_bg_fill = panel;
        // 按钮三态必须有明显区分：静止=panel2，悬停=hover，按压=active。
        // 注意 egui Button 悬停时读的是 hovered 状态的 bg_fill/weak_bg_fill，
        // 三态若填同一个颜色按钮就会"看起来不可点"。
        v.widgets.inactive.bg_fill = panel2;
        v.widgets.inactive.weak_bg_fill = panel2;
        v.widgets.hovered.bg_fill = hex(t.hover);
        v.widgets.hovered.weak_bg_fill = hex(t.hover);
        v.widgets.hovered.bg_stroke.color = hex(t.border);
        v.widgets.active.bg_fill = hex(t.active);
        v.widgets.active.weak_bg_fill = hex(t.active);
        ctx.set_visuals(v);
    }

    /// 应用字体尺寸到 egui 上下文（中文字体已在启动时注册）。
    pub fn apply_text_styles(&self, ctx: &egui::Context) {
        let f = egui::FontFamily::Proportional;
        ctx.style_mut(|s| {
            s.text_styles = [
                (TextStyle::Body, FontId::new(self.font_size_body, f.clone())),
                (TextStyle::Small, FontId::new(self.font_size_small, f.clone())),
                (TextStyle::Heading, FontId::new(self.font_size_heading, f.clone())),
                (TextStyle::Button, FontId::new(self.font_size_body, f.clone())),
                (
                    TextStyle::Monospace,
                    FontId::new(self.font_size_monospace, egui::FontFamily::Monospace),
                ),
            ]
            .into();
        });
    }

    /// F12 调试面板：全部参数滑块实时微调 + 保存 / 恢复默认。
    pub fn debug_panel(&mut self, ctx: &egui::Context, show: &mut bool, dirty: &mut bool) {
        if !*show {
            return;
        }
        // Window::open 与闭包同时借 show 会冲突：先在局部开关上操作，结束后写回
        let mut open_flag = *show;
        let mut force_close = false;
        egui::Window::new("界面微调 (ui_style.json)")
            .open(&mut open_flag)
            .default_width(320.0)
            .show(ctx, |ui| {
                egui::Grid::new("style_grid")
                    .num_columns(2)
                    .spacing([8.0, self.row_spacing * 0.5])
                    .show(ui, |ui| {
                        macro_rules! drag {
                            ($label:expr, $field:expr, $range:expr) => {{
                                ui.label($label);
                                let before = $field;
                                ui.add(
                                    egui::DragValue::new(&mut $field)
                                        .speed(0.5)
                                        .range($range),
                                );
                                if (before - $field).abs() > f32::EPSILON {
                                    *dirty = true;
                                }
                                ui.end_row();
                            }};
                        }
                        drag!("左栏宽度", self.left_panel_width, 300.0..=600.0);
                    });

                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("保存到 ui_style.json").clicked() {
                        match self.save() {
                            Ok(_) => *dirty = false,
                            Err(e) => {
                                force_close = true;
                                eprintln!("样式保存失败: {}", e);
                            }
                        }
                    }
                    if ui.button("恢复默认").clicked() {
                        *self = StyleConfig::default();
                        *dirty = true;
                    }
                });
                if *dirty {
                    ui.colored_label(egui::Color32::YELLOW, "● 有未保存的修改");
                }
                ui.small("保存后下次启动自动加载；不同电脑可用各自的 ui_style.json");
            });
        *show = open_flag && !force_close;
    }
}

/// 8 通道曲线颜色（与 Python 版 CHANNEL_COLORS 对齐的亮色系）。
pub const CHANNEL_COLORS: [Color32; 8] = [
    Color32::from_rgb(0xE6, 0x61, 0x57), // 红
    Color32::from_rgb(0x5C, 0xA6, 0x32), // 绿
    Color32::from_rgb(0x3E, 0x8E, 0xED), // 蓝
    Color32::from_rgb(0xFB, 0xBD, 0x2F), // 黄
    Color32::from_rgb(0x95, 0x75, 0xCD), // 紫
    Color32::from_rgb(0x2B, 0xB5, 0xCE), // 青
    Color32::from_rgb(0xF0, 0x8C, 0x3C), // 橙
    Color32::from_rgb(0xD1, 0x69, 0xD1), // 品红
];

/// 注册中文字体（优先系统微软雅黑，兜底 Noto/苹方）。
pub fn install_fonts(ctx: &egui::Context) {
    let candidates = [
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\msyh.ttf",
        r"C:\Windows\Fonts\simhei.ttf",
        "/System/Library/Fonts/PingFang.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    ];
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "cjk".into(),
                egui::FontData::from_owned(bytes),
            );
            // 插到最前：中文用 CJK 字体渲染，拉丁字符会回退到 egui 默认（proportional 顺序）
            if let Some(fam) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                fam.insert(0, "cjk".into());
            }
            if let Some(fam) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                fam.push("cjk".into()); // 等宽族末尾兜底中文
            }
            ctx.set_fonts(fonts);
            return;
        }
    }
    // 找不到就保持默认（界面英文可显示，中文为方框）
    eprintln!("未找到中文字体，界面中文将显示为方框");
}
