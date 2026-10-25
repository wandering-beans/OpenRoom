use advanced_explorer_develop::{self as engine, BaseInfo, Settings, WorkingImage};
use std::time::Instant;
fn measure(mut operation: impl FnMut(), count: usize) -> (f64, f64) {
    operation();
    let mut timings = Vec::new();
    for _ in 0..count {
        let started = Instant::now();
        operation();
        timings.push(started.elapsed().as_secs_f64() * 1000.);
    }
    timings.sort_by(f64::total_cmp);
    (
        timings[count / 2],
        timings[(count * 95 / 100).min(count - 1)],
    )
}
fn main() -> anyhow::Result<()> {
    rayon::ThreadPoolBuilder::new().num_threads(4).build()?.install(|| -> anyhow::Result<()> {
        let image=image::Rgba32FImage::from_fn(1600,1067,|x,y| image::Rgba([x as f32/2000.,y as f32/1500.,0.2,1.]));
        let base=WorkingImage{pixels:image,info:BaseInfo{camera_saturation:None,native_source:None,raster_wb_temperature:None,width:1600,height:1067,source_width:6000,source_height:4000,source_kind:"synthetic".into(),color_note:"linear sRGB".into(),raw:None}};
        let typical=Settings{exposure:0.5,contrast:12.,highlights:-25.,shadows:20.,vibrance:15.,..Default::default()};
        let detail=Settings{sharpen:40.,texture:15.,clarity:10.,noise_luma:15.,..typical.clone()};
        let mut renders=Vec::new();
        for (name, settings) in [("neutral",Settings::default()),("tone_color",typical),("tone_color_detail",detail)] {
            let (median,p95)=measure(||{std::hint::black_box(engine::render(&base,&settings,&||false).unwrap());},20);
            renders.push(serde_json::json!({"settings":name,"median_ms":median,"p95_ms":p95}));
        }
        let large=image::Rgba32FImage::from_pixel(4096,2731,image::Rgba([0.1,0.3,0.8,1.]));
        let (fir_median,fir_p95)=measure(||{std::hint::black_box(engine::resize_linear(large.clone(),1600).unwrap());},10);
        let (image_median,image_p95)=measure(||{std::hint::black_box(image::imageops::resize(&large,1600,1067,image::imageops::FilterType::Lanczos3));},10);
        println!("{}",serde_json::to_string_pretty(&serde_json::json!({"threads":4,"preview":[1600,1067],"renders":renders,"resize":{"source":[4096,2731],"destination":[1600,1067],"fir":{"median_ms":fir_median,"p95_ms":fir_p95},"image":{"median_ms":image_median,"p95_ms":image_p95}}}))?);
        Ok(())
    })
}
