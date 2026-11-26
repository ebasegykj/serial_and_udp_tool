//! 绘图区：egui_plot 实时曲线 + 缩放/复位支持。

use egui_plot::{
    Corner, Legend, Line, LineStyle, MarkerShape, Plot, PlotBounds, PlotPoints, Points, VLine,
};

use super::i18n::{tr, Lang};
use super::style::CHANNEL_COLORS;

/// 绘制全部可见通道曲线。
///
/// `series`: (通道名, 数值序列, 通道色序号, x 基准)。
/// - `Some(name)`：常规曲线，带名字 → 进入图例（对齐 Python `label="CHn"`）。
/// - `None`：叠加曲线（如差分线），无名字 → 不进图例（对齐 Python `label='_nolegend_'`），
///   用虚线区分原始实线。
/// - `x 基准`：真实帧序号偏移——缓冲只存最近 N 个点，第 i 个点的 x = 基准 + i，
///   使坐标轴与鼠标读数均为真实帧号（v16 用户要求）。
/// x 轴 = 真实帧序号；`height`: 绘图区固定高度（像素），保证下方发送行贴底。
/// `epoch`: 复位计数——变化时换 Plot id，egui 自动回到自适应边界（等价 matplotlib 的 Home）。
/// `zoom`: 一次性缩放因子（1.0 = 无操作；<1 放大，>1 缩小），执行后归位。
/// `show_points`: 暂停时为 true —— 在原始曲线上叠加圆形标记点突出数据点
/// （Rust 版新增；差分虚线是辅助线不打点，标记点不进图例）。
#[allow(clippy::too_many_arguments)]
pub fn draw_plot(
    ui: &mut egui::Ui,
    line_width: f32,
    series: &[(Option<String>, Vec<f64>, usize, f64)],
    height: f32,
    epoch: u64,
    zoom: &mut f32,
    show_points: bool,
    lang: Lang,
) -> Option<[f64; 2]> {
    draw_plot_with_id(
        "data_plot", ui, line_width, series, height, epoch, zoom, show_points, None, None,
        &mut false, lang,
    )
}

/// 从序列真实点（含首尾透明辅助点撑开的 x 范围）与异常标记算出数据真实范围。
/// 返回 [min_x, max_x, min_y, max_y]；无有效点时 None。
/// 只在「强制重置坐标」那一帧调用，用于 set_plot_bounds 一次性设定真实边界。
pub fn compute_series_bounds(
    series: &[(Option<String>, Vec<f64>, usize, f64)],
    x_bounds: Option<[f64; 2]>,
    marker: Option<(f64, Vec<f64>)>,
) -> Option<[f64; 4]> {
    let (mut min_x, mut max_x) = (f64::INFINITY, f64::NEG_INFINITY);
    let (mut min_y, mut max_y) = (f64::INFINITY, f64::NEG_INFINITY);
    for (_, vals, _, x0) in series.iter() {
        for (i, v) in vals.iter().enumerate() {
            let x = x0 + i as f64;
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(*v);
            max_y = max_y.max(*v);
        }
    }
    // 透明辅助点的 y 取首/末真实值，已含在 raw 的 Y 范围里；只需并入它的 x 范围。
    if let Some([lo, hi]) = x_bounds {
        min_x = min_x.min(lo);
        max_x = max_x.max(hi);
    }
    if let Some((x, ys)) = marker {
        min_x = min_x.min(x);
        max_x = max_x.max(x);
        for y in ys {
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
    }
    (min_x.is_finite() && max_x.is_finite() && min_y.is_finite() && max_y.is_finite())
        .then_some([min_x, max_x, min_y, max_y])
}

/// 把真实范围转成重置帧要设的边界：X 严格等于数据范围（前/后帧数窗口），
/// Y 上下各留 5% 边距（全平时至少 1.0）避免曲线贴边。
pub fn bounds_with_y_margin(real: [f64; 4]) -> [[f64; 2]; 2] {
    let [min_x, max_x, min_y, max_y] = real;
    let span_y = (max_y - min_y).max(1.0);
    [[min_x, min_y - span_y * 0.05], [max_x, max_y + span_y * 0.05]]
}

/// 给快照曲线首尾补 x=lo / x=hi 的边界点（y 取该通道首/末值），
/// 让 egui_plot 的**自动边界**自然覆盖整个窗口且 Y 按真实值自适应。
/// 注意：这些补点只用于「撑开坐标范围」，绘制时走全透明辅助线，不会画成可见的平延长线；
/// 真正可见的曲线只用原始真实点（见 draw_plot_with_id）。
/// 关键：绝不在闭包里 set_plot_bounds —— 那里的 plot_bounds 还是默认 Y=-1~0，
/// 会把真实曲线裁掉，且覆盖用户的滚轮缩放/拖动结果。
pub fn pad_points_to_x_range(pts: &[[f64; 2]], range: [f64; 2]) -> Vec<[f64; 2]> {
    let mut out = pts.to_vec();
    let (lo, hi) = (range[0], range[1]);
    if let Some(first) = out.first().copied() {
        if first[0] > lo {
            out.insert(0, [lo, first[1]]);
        }
    }
    if let Some(last) = out.last().copied() {
        if last[0] < hi {
            out.push([hi, last[1]]);
        }
    }
    out
}

/// 与主绘图区同款曲线，但用独立 Plot id（验算弹窗里画快照，不与主图抢 id/边界）。
/// `marker`: Some((x, 各通道在 x 处的 y))，画红色竖线 + 大红圈标记异常点。
/// `x_bounds`: 部分场景下给序列首尾补边界点，使自动边界覆盖该 x 范围（不锁 Y、不抢缩放）。
#[allow(clippy::too_many_arguments)]
pub fn draw_plot_with_id(
    id_base: &str,
    ui: &mut egui::Ui,
    line_width: f32,
    series: &[(Option<String>, Vec<f64>, usize, f64)],
    height: f32,
    epoch: u64,
    zoom: &mut f32,
    show_points: bool,
    marker: Option<(f64, Vec<f64>)>,
    x_bounds: Option<[f64; 2]>,
    // 为 true 的那一帧强制把坐标范围设成数据真实范围，用后自动置回 false。
    // 仅此一帧调用 set_plot_bounds，其余帧不碰，用户缩放/拖动不会被覆盖。
    reset_bounds: &mut bool,
    lang: Lang,
) -> Option<[f64; 2]> {
    let mut coord: Option<[f64; 2]> = None;
    let plot = Plot::new((id_base.to_owned(), epoch))
        // 图例固定左上（与 matplotlib 无数据时的默认位置观感一致）
        .legend(Legend::default().position(Corner::LeftTop))
        // 验算图关掉那层 extreme_bg_color 实心矩形，透出下方已半透明的窗口底板；
        // 主图（data_plot）保留背景，行为完全不变。只关 Rect，网格/曲线/图例/轴仍正常。
        .show_background(id_base != "verify_plot")
        .allow_drag(true)
        .allow_zoom(true)
        .allow_scroll(true)
        .height(height)
        .x_axis_label(tr(lang, "plot_x_label"))
        .y_axis_label(tr(lang, "plot_y_label"));
    plot.show(ui, |pui| {
        // 坐标提示只在鼠标悬停于绘图区内时显示（egui_plot 的 pointer_coordinate
        // 不做区域判定，鼠标在窗口其他位置也会换算出错误的"坐标"），
        // 且坐标本身就是绘图坐标（x=帧序号，y=通道值），非屏幕像素位置。
        coord = if pui.response().hovered() {
            pui.pointer_coordinate().map(|p| [p.x, p.y])
        } else {
            None
        };
        if *reset_bounds {
            // 重置帧：跳过缩放分支，下面用数据真实范围一次性设定边界。
            *zoom = 1.0;
        } else if (*zoom - 1.0).abs() > f32::EPSILON {
            let b = pui.plot_bounds();
            let (min, max) = (b.min(), b.max());
            let cx = (min[0] + max[0]) * 0.5;
            let cy = (min[1] + max[1]) * 0.5;
            let hw = (max[0] - min[0]) * 0.5 * *zoom as f64;
            let hh = (max[1] - min[1]) * 0.5 * *zoom as f64;
            pui.set_plot_bounds(PlotBounds::from_min_max(
                [cx - hw, cy - hh],
                [cx + hw, cy + hh],
            ));
            *zoom = 1.0;
        }
        for (name, vals, color_idx, x0) in series.iter() {
            // 空通道也要注册图例项（matplotlib 空线也显示 CH 名，无数据时图例可见）
            // x = 真实帧序号：缓冲偏移基准 + 序号
            let raw: Vec<[f64; 2]> = vals
                .iter()
                .enumerate()
                .map(|(i, v)| [x0 + i as f64, *v as f64])
                .collect();
            // 验算快照：边界点只进「全透明辅助线」用于撑开自动边界，绝不画成可见线。
            // 可见曲线/数据点只用 raw（真实帧号点），消除开头帧左侧那条平延水平线。
            // 不用 NaN 断开：egui_plot 0.29 Line::shapes 不过滤 NaN 顶点，透明辅助线更可靠。
            if let Some(range) = x_bounds {
                let bounds_pts = pad_points_to_x_range(&raw, range);
                pui.line(
                    Line::new(PlotPoints::from(bounds_pts))
                        .color(egui::Color32::TRANSPARENT)
                        .width(0.0_f32)
                        .name(""),
                );
            }
            let pts = raw;
            let color = CHANNEL_COLORS[color_idx % CHANNEL_COLORS.len()];
            let mut line = Line::new(PlotPoints::from(pts.clone()))
                .color(color)
                .width(line_width);
            match name {
                Some(n) => {
                    // 原始实线：带名字进图例
                    line = line.name(n.clone());
                    // 暂停时突出数据点：原始曲线叠加实心圆点（同通道色、不进图例）
                    if show_points && !pts.is_empty() {
                        pui.points(
                            Points::new(PlotPoints::from(pts.clone()))
                                .shape(MarkerShape::Circle)
                                .color(color)
                                .filled(true)
                                .radius(line_width * 1.5 + 2.0),
                        );
                    }
                }
                None => {
                    // 差分虚线：不进图例（Python `_nolegend_` 等价）
                    line = line.style(LineStyle::dashed_dense());
                }
            }
            pui.line(line);
        }
        // 异常点标注：红色竖线 + 大号空心红圈（不盖住数据点本身），图例名 Anomaly。
        if let Some((x, ref ys)) = marker {
            let red = egui::Color32::from_rgb(0xE0, 0x40, 0x40);
            pui.vline(VLine::new(x).color(red).width(2.0_f32));
            let marks: Vec<[f64; 2]> = ys.iter().map(|&y| [x, y]).collect();
            pui.points(
                Points::new(PlotPoints::from(marks))
                    .shape(MarkerShape::Circle)
                    .color(red)
                    .filled(false)
                    .radius(line_width * 3.0 + 3.0)
                    .name(tr(lang, "plot_anomaly")),
            );
        }
        // 强制重置帧：按数据真实范围一次性设边界（X 严格、Y 留边距），用后置回 false。
        // egui_plot 会把 set_plot_bounds 写进 PlotMemory 并关掉 auto_bounds，
        // 因此只有这一帧改边界；其余帧保留用户的滚轮缩放/拖动结果。
        if *reset_bounds {
            if let Some(real) = compute_series_bounds(series, x_bounds, marker) {
                let [min, max] = bounds_with_y_margin(real);
                pui.set_plot_bounds(PlotBounds::from_min_max(min, max));
                // 只有真正设了边界才消费标志；无数据的帧不消耗，下一帧继续重试，
                // 避免"空帧吃掉重置"导致换条目后图仍停在旧/默认边界。
                *reset_bounds = false;
            }
        }
    });
    coord
}
