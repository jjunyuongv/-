//! 7단계 확인용: 개발용 이미지 1장 인식 → 줄 수·시간·라벨 대비 PII 값 포함 여부 (값은 찍지 않음)
//! cargo run --release -p ocr --example ocr_one -- data/dev_s41/s41_0005_p1.jpg [모델 폴더]
use std::path::PathBuf;
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let img = PathBuf::from(args.get(1).ok_or("이미지 경로")?);
    let dir = PathBuf::from(args.get(2).map_or("models/ocr/monkt", String::as_str));
    let t = Instant::now();
    let engine = ocr::OcrEngine::new(&ocr::OcrModels::in_dir(&dir))?;
    let load = t.elapsed();
    let image = image::open(&img)?.to_rgb8();
    let t = Instant::now();
    let page = engine.recognize(image)?;
    let run = t.elapsed();
    let text: String = page
        .lines
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    println!(
        "모델 로딩 {:.2}s, 인식 {:.2}s, 줄 {}개, 글자 {}개",
        load.as_secs_f64(),
        run.as_secs_f64(),
        page.lines.len(),
        text.chars().count()
    );
    // 라벨 JSON이 있으면 PII 값이 OCR 텍스트(공백 무시)에 온전히 나오는지 센다
    let label = img.with_file_name(format!(
        "{}.json",
        img.file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .rsplit_once("_p")
            .map_or("", |x| x.0)
    ));
    if let Ok(src) = std::fs::read_to_string(&label) {
        let v: serde_json::Value = serde_json::from_str(&src)?;
        let flat: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        let (mut hit, mut total) = (0, 0);
        let page_spans: std::collections::BTreeSet<u64> = v["boxes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|b| b["page"] == 0)
            .filter_map(|b| b["span_id"].as_u64())
            .collect();
        for s in v["spans"].as_array().into_iter().flatten() {
            if !page_spans.contains(&s["id"].as_u64().unwrap_or(0)) {
                continue;
            }
            total += 1;
            let val: String = s["value"]
                .as_str()
                .unwrap_or("")
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            if flat.contains(&val) {
                hit += 1;
            }
        }
        println!("1쪽 PII 값 인식: {hit} / {total}");
    }
    Ok(())
}
