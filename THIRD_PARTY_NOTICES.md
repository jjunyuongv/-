# THIRD_PARTY_NOTICES

가려냄이 직접 의존하는 외부 크레이트·모델과 라이선스 목록. 추가할 때마다 갱신한다.
AGPL은 사용하지 않는다 (CLAUDE.md 라이선스 규칙).

## Rust 크레이트 (직접 의존)

| 크레이트 | 버전 | 라이선스 | 사용처 | 추가일 |
|---|---|---|---|---|
| `clap` | 4.6.7 | MIT OR Apache-2.0 | `apps/cli` 인자 파싱 | 2026-11-07 |
| `anyhow` | 1.0.104 | MIT OR Apache-2.0 | `apps/cli` 오류 처리 | 2026-11-07 |
| `pdfium-render` | 0.9.5 | MIT OR Apache-2.0 | `apps/cli` `forms extract`: PDF 텍스트·글자 위치 추출, 쪽 렌더링. `synth` 테스트(dev): D0 PDF를 다시 읽어 라벨과 대조 | 2026-11-08 |
| `image` | 0.25.10 | MIT OR Apache-2.0 | `apps/cli` PNG 저장, `synth` 열화 이미지 저장 (`png`·`jpeg` 기능만) | 2026-11-08 |
| `serde` | 1.0.229 | MIT OR Apache-2.0 | `apps/cli` 추출 결과 JSON, `pii-core` 라벨 타입, `synth` 템플릿 | 2026-11-08 |
| `serde_json` | 1.0.154 | MIT OR Apache-2.0 | `apps/cli` 추출 결과 JSON, `synth` 문장 풀 JSONL (`pii-core`는 테스트에만) | 2026-11-08 |
| `toml` | 0.9.8 | MIT OR Apache-2.0 | `synth` 양식 템플릿(`assets/templates/*.toml`) 읽기 | 2026-11-08 |
| `thiserror` | 2.0.21 | MIT OR Apache-2.0 | `synth` 오류 타입 (라이브러리 크레이트 규칙) | 2026-11-08 |
| `rand` | 0.10.3 | MIT OR Apache-2.0 | `synth` 가짜 PII 생성 (기본 기능 끔) | 2026-11-08 |
| `rand_chacha` | 0.10.0 | MIT OR Apache-2.0 | `synth` 시드 고정 RNG (`ChaCha8Rng`) | 2026-11-08 |
| `ab_glyph` | 0.2.32 | Apache-2.0 | `synth` 글자 래스터화 (렌더링) | 2026-11-08 |
| `tiny-skia` | 0.12.0 | BSD-3-Clause | `synth` 선·표·서명·도장 그리기, PNG 저장 | 2026-11-08 |
| `krilla` | 0.8.2 | MIT OR Apache-2.0 | `synth` D0 텍스트 레이어 PDF 생성 (`raster-images` 기능만, 글꼴 부분 집합 내장) | 2026-11-08 |
| `oar-ocr` | 0.10.2 | Apache-2.0 | `ocr` PP-OCRv5 검출·인식 파이프라인 (기본 기능: ONNX Runtime 바이너리 내려받기·SIMD). 간접 의존 `oar-ocr-core`·`imageproc` 0.27(MIT)·`image` 전체 코덱 | 2026-11-08 |
| `unicode-normalization` | 0.1.26 | (MIT OR Apache-2.0) AND Unicode-3.0 | `pii-core` 탐지용 NFKC 정규화 | 2026-11-08 |
| `regex` | 1.13.1 | MIT OR Apache-2.0 | `rulepack`·`detect-rules` 규칙 팩 정규식 (선형 시간) | 2026-11-08 |
| `ort` | 2.0.0-rc.13 | MIT OR Apache-2.0 | `detect-ner` KoELECTRA ONNX 실행 (`oar-ocr`과 같은 버전) | 2026-11-08 |
| `tokenizers` | 0.23.2 | Apache-2.0 | `detect-ner` 토크나이저 (`fancy-regex`만, 글자 오프셋) | 2026-11-08 |
| `reqwest` | 0.13.5 | MIT OR Apache-2.0 | `detect-slm` llama-server 호출 (blocking·json, 로컬 HTTP) | 2026-11-08 |

> 간접 의존성까지 포함한 전체 고지는 배포 전에 `cargo about` 등으로 생성한다.

## 네이티브 라이브러리 (동적 로딩)

| 이름 | 버전 | 라이선스 | 위치 | 사용처 | 추가일 |
|---|---|---|---|---|---|
| PDFium (`pdfium.dll`, [bblanchon/pdfium-binaries](https://github.com/bblanchon/pdfium-binaries) 빌드) | chromium/8125 (`pdfium-win-x64`, V8·XFA 없음) | PDFium: BSD-3-Clause / 빌드 스크립트: MIT / 포함 라이브러리(FreeType, ICU, libjpeg-turbo, OpenJPEG, libpng, zlib, lcms, abseil, HarfBuzz 등): 각 허용형 라이선스 — 원문은 `vendor/pdfium/licenses/` | `vendor/pdfium/bin/` | `pdfium-render`가 실행 시 로딩 | 2026-11-08 |
| ONNX Runtime (`ort` 2.0.0-rc.13이 빌드 때 내려받아 링크) | `ort` 크레이트 기본값 | ONNX Runtime: MIT / `ort`: MIT OR Apache-2.0 | 빌드 산출물에 포함 | `oar-ocr`·(10단계) NER 실행 | 2026-11-08 |
| llama.cpp (`llama-server.exe` 등, Windows Vulkan 빌드) | b12202 (`llama-b12202-bin-win-vulkan-x64.zip`) | MIT (포함 LLVM OpenMP: Apache-2.0 with LLVM exception) | `vendor/llama.cpp/b12202/` | SLM 실행 (별도 프로세스, OpenAI 호환 API) | 2026-11-08 |

- AGPL 조항 없음 확인 (`vendor/pdfium/LICENSE`, `vendor/pdfium/licenses/*` 검색, 2026-11-08)
- `pdfium-render 0.9.5`의 기본 기능 `pdfium_latest` = PDFium API 8125 → DLL도 같은 번호로 고정

## 폰트 (`assets/fonts/`, 합성 이미지 렌더링)

| 폰트 | 파일 | 라이선스 | 출처 | 추가일 |
|---|---|---|---|---|
| 나눔고딕 (Regular·Bold) | `NanumGothic-*.ttf` | SIL OFL 1.1 (`OFL-Nanum.txt`) | Google Fonts 저장소 (google/fonts, ofl/) | 2026-11-08 |
| 나눔명조 | `NanumMyeongjo-Regular.ttf` | SIL OFL 1.1 | 같음 | 2026-11-08 |
| 나눔손글씨 펜 | `NanumPenScript-Regular.ttf` | SIL OFL 1.1 | 같음 | 2026-11-08 |
| 나눔손글씨 붓 | `NanumBrushScript-Regular.ttf` | SIL OFL 1.1 | 같음 | 2026-11-08 |
| Noto Sans CJK KR (한국어 부분 집합, Regular) — 나눔 글꼴에 없는 글자 대체(㎡ 등) | `NotoSansKR-Regular.otf` | SIL OFL 1.1 (`OFL-NotoSansCJK.txt`) | [notofonts/noto-cjk](https://github.com/notofonts/noto-cjk) `Sans/SubsetOTF/KR/` | 2026-11-08 |
| Noto Sans (Regular) — CJK에도 없는 문장 부호 대체(U+2024) | `NotoSans-Regular.ttf` | SIL OFL 1.1 (`OFL-NotoSans.txt`) | [notofonts/latin-greek-cyrillic](https://github.com/notofonts/latin-greek-cyrillic) (notofonts.github.io 배포본, hinted) | 2026-11-08 |

## 합성용 자료 출처 (`assets/synth/`)

| 파일 | 내용 | 출처·근거 |
|---|---|---|
| `names_ko.toml` | 성씨 비율, 세대별 이름 음절 | 성씨: 통계청 2015 인구주택총조사 성씨 통계 근사치. 이름 음절: 작성자 구성 (실제 인명 목록 아님) |
| `addresses_ko.toml` | 시·군·구별 도로명 + 그 도로의 법정동·읍·면 짝, 최대 건물본번 | **행정안전부 도로명주소 한글 전체분 TXT** (도로명주소 안내시스템 juso.go.kr 공개 자료, 다운로드 2026-11-08)에서 30개 시·군·구 발췌. 원본은 `assets/synth/source/juso_20261108/` — **개발용 자료, 앱에 넣지 않음, git 제외**. 발췌 규칙은 TOML 머리에 기록 |
| `foreign.toml` | 국가별 이름 조각·순서, 여권·주소 형식 | 작성자 구성 (흔한 이름 조각, 형식 근사치) |
| `banks.toml` | 은행별 계좌번호 형식 | 널리 알려진 표기 형식 근사치 |

## 모델 (`models/`, git 제외)

| 모델 | 파일 | 크기 | 라이선스 | 출처 | 기기 배포 | 추가일 |
|---|---|---:|---|---|---|---|
| PP-OCRv5 server 검출 (ONNX 변환본) | `models/ocr/monkt/det.onnx` | 84MB | Apache-2.0 | [monkt/paddleocr-onnx](https://huggingface.co/monkt/paddleocr-onnx) `detection/v5/` | 가능 (고지) | 2026-11-08 |
| PP-OCRv5 한국어 인식 + 사전 | `models/ocr/monkt/rec.onnx`, `dict.txt` | 13MB | Apache-2.0 | 같음 `languages/korean/` | 가능 (고지) | 2026-11-08 |
| PP-OCRv5 한국어 사전 (대체 경로용, 사용 안 함) | `models/ocr/oar/ppocrv5_korean_dict.txt` | 46KB | Apache-2.0 | GreatV/oar-ocr 릴리스 v0.3.0 | — | 2026-11-08 |
| KoELECTRA 한국어 PII NER (INT8 ONNX) | `models/ner/koelectra-ko-pii-ner/model_int8.onnx`, `tokenizer.json`, `config.json` | 15MB | Apache-2.0 | [atonlee/koelectra-ko-pii-ner](https://huggingface.co/atonlee/koelectra-ko-pii-ner) | 가능 (고지, 개인 업로드 모델) | 2026-11-08 |
| Qwen3.5-4B Q4_K_M GGUF | `models/slm/Qwen3.5-4B-Q4_K_M.gguf` | 2.7GB | Apache-2.0 | [unsloth/Qwen3.5-4B-GGUF](https://huggingface.co/unsloth/Qwen3.5-4B-GGUF) (원본 Qwen/Qwen3.5-4B) | 가능 (고지) | 2026-11-08 |
