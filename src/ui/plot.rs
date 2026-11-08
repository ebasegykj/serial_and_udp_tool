//! 绘图区：egui_plot 实时曲线 + 缩放/复位支持。

use egui_plot::{
    Corner, Legend, Line, LineStyle, MarkerShape, Plot, PlotBounds, PlotPoints, Points,
};

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
) -> Option<[f64; 2]> {
    let mut coord: Option<[f64; 2]> = None;
    let plot = Plot::new(("data_plot", epoch))
        // 图例固定左上（与 matplotlib 无数据时的默认位置观感一致）
        .legend(Legend::default().position(Corner::LeftTop))
        .allow_drag(true)
        .allow_zoom(true)
        .allow_scroll(true)
        .height(height)
        .x_axis_label("Frame Number")
        .y_axis_label("Value");
    plot.show(ui, |pui| {
        // 坐标提示只在鼠标悬停于绘图区内时显示（egui_plot 的 pointer_coordinate
        // 不做区域判定，鼠标在窗口其他位置也会换算出错误的"坐标"），
        // 且坐标本身就是绘图坐标（x=帧序号，y=通道值），非屏幕像素位置。
        coord = if pui.response().hovered() {
            pui.pointer_coordinate().map(|p| [p.x, p.y])
        } else {
            None
        };
        if (*zoom - 1.0).abs() > f32::EPSILON {
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
            let pts: Vec<[f64; 2]> = vals
                .iter()
                .enumerate()
                .map(|(i, v)| [x0 + i as f64, *v as f64])
                .collect();
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
    });
    coord
}
