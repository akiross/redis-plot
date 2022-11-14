// TODO make this an enum that differentiates required data for each plot.
// Right now, this is simply for line plots.
pub struct PlotSpec {
    pub color: Vec<(u8, u8, u8)>,
    pub data: Vec<Vec<(f32, f32)>>,
    pub bg_color: (u8, u8, u8),
}

/// This function plots a complex plot onto a backend.
pub fn plot_complex<DB>(
    root: plotters::drawing::DrawingArea<DB, plotters::coord::Shift>,
    spec: PlotSpec,
) where
    DB: plotters_backend::DrawingBackend,
{
    use plotters::prelude::*;
    root.fill(&RGBColor(spec.bg_color.0, spec.bg_color.1, spec.bg_color.2))
        .unwrap();
    //let root = root.margin(25, 25, 25, 25);

    let (x_range, y_range) = {
        let mut x_min: Option<f32> = None;
        let mut x_max: Option<f32> = None;
        let mut y_min: Option<f32> = None;
        let mut y_max: Option<f32> = None;

        for data in spec.data.iter() {
            for (x, y) in data.iter() {
                if x < x_min.get_or_insert(*x) {
                    x_min.replace(*x);
                }
                if x > x_max.get_or_insert(*x) {
                    x_max.replace(*x);
                }
                if y < y_min.get_or_insert(*y) {
                    y_min.replace(*y);
                }
                if y > y_max.get_or_insert(*y) {
                    y_max.replace(*y);
                }
            }
        }
        if x_min.is_none() || x_max.is_none() || y_min.is_none() || y_max.is_none() {
            (0.0..0.0, 0.0..0.0)
        } else {
            (
                x_min.unwrap()..x_max.unwrap(),
                y_min.unwrap()..y_max.unwrap(),
            )
        }
    };

    let mut chart = ChartBuilder::on(&root)
        .margin(25i32)
        .x_label_area_size(30u32)
        .y_label_area_size(30u32)
        .caption("RSPlotters", ("sans-serif", 20u32))
        .build_cartesian_2d(x_range, y_range)
        .unwrap();

    chart.configure_mesh().draw().unwrap();

    // Draw all the lines
    for (data, color) in spec.data.into_iter().zip(spec.color.iter()) {
        let color = &RGBColor(color.0, color.1, color.2);
        chart.draw_series(LineSeries::new(data, color)).unwrap();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
