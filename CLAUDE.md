**모든 답변·보고·문서는 반드시 한국어로 작성한다. 영어로 답하지 않는다.**

# 가려냄 — 근로계약서 개인정보 마스킹 PoC

근로계약서 이미지·PDF의 개인정보를 **기기 안에서** 가릴 수 있는지 수치로 판정하는 PoC.
최종 서비스도 Rust로 만든다. 상세 계획·결정 사항·판정 기준은 **PLAN.md**가 기준이다.

## 작업 방식
- PLAN.md의 단계 순서대로 진행한다. 단계를 시작하기 전에 PLAN.md의 해당 섹션을 다시 읽는다.
- 끝낸 항목은 PLAN.md 체크박스(`- [x]`)를 업데이트한다. 계획과 다르게 진행했으면 이유를 그 항목 아래에 한 줄로 남긴다.
- PLAN.md에 "결정 필요"로 남은 항목이나 계획에 없는 선택(새 크레이트·모델 추가, 구조 변경)은 진행 전에 사용자에게 묻는다.
- **체크포인트에서는 멈추고 결과를 보여준 뒤 확인을 받는다**: 1-1 PDF 추출 결과, 4단계 3건 렌더링(박스 오버레이), 7단계 oar-ocr 호환·SLM 실측, 13단계 판정.
- 답변, 문서, 코드 주석은 **한국어**. 식별자(변수·함수·타입·크레이트 이름)는 **영어**.
- 작업은 **Claude Code CLI**와 **VS Code의 Claude 채팅** 두 곳에서 번갈아 한다. 대화 내용은 세션끼리 공유되지 않으므로:
  - 진행 상황·결정·남은 문제는 반드시 **PLAN.md에 기록**한다 (체크박스 + 한 줄 메모). 새 세션은 PLAN.md를 읽고 마지막 체크된 항목 다음부터 이어간다.
  - 한 세션에서 시작한 작업은 그 세션에서 빌드·테스트 통과까지 마무리하고 넘긴다. 못 끝냈으면 PLAN.md 해당 항목 아래에 `(진행 중: …)`으로 남긴다.

## 환경
- Windows 11, 터미널 = **cmd (명령 프롬프트)**. 사용자에게 안내하는 명령어는 **cmd 문법**으로 쓴다.
  - Claude가 도구로 직접 실행하는 셸은 cmd가 아닐 수 있다 (예: Git Bash). 직접 실행할 때는 그 셸 문법을 쓰되, 사용자에게 알려주는 명령은 cmd로 바꿔서 쓴다.
  - 한글 출력이 깨지면 `chcp 65001`
  - 환경변수: 현재 창은 `set 이름=값`, 영구는 `setx 이름 "값"` (단, PATH는 `setx`로 바꾸지 말고 "계정의 환경 변수 편집" 화면에서)
- 작업 폴더: `C:\DevData\가려냄` (한글 경로). 순수 Rust 빌드는 문제없음. C/C++ 빌드가 섞인 크레이트가 경로 오류를 내면 `mklink /J C:\dev\garyeonaem C:\DevData\가려냄`로 우회.
- Rust stable (`x86_64-pc-windows-msvc`). Python은 `tools/python/`의 모델 변환·학습에만 쓴다.
- 노트북: Core Ultra 7 258V / RAM 32GB / Intel Arc 140V 내장 GPU. **CUDA 없음** → GPU는 Vulkan(llama.cpp), DirectML·OpenVINO(ONNX Runtime)로만.
- git 원격: `https://github.com/jjunyuongv/-.git` (**공개 저장소**). `.gitignore`로 `data/`·`models/`·`vendor/`·`assets/forms/`·`assets/synth/source/`·`results/`를 제외한다 — 합성 데이터·리포트를 올리지 않는다 (PLAN.md 8장).
- PDF 처리(`pdfium-render`)는 `vendor\pdfium\bin\pdfium.dll`(chromium/8125)이 필요하다. 없으면 PLAN.md 1-1 "pdfium 준비"대로 받는다. 다른 곳에 두면 `set PDFIUM_DYNAMIC_LIB_PATH=폴더경로`.

## 코드 규칙
- Cargo workspace 구조는 PLAN.md 2장을 따른다 (`crates/*`, `apps/cli`).
- 라이브러리 크레이트는 `thiserror`, 앱(`apps/cli`)은 `anyhow`. 라이브러리 코드에서 `unwrap()`/`expect()` 금지 (테스트 제외).
- 랜덤은 시드 고정 (`rand_chacha`) — 같은 시드면 같은 데이터가 나와야 한다.
- 작업을 마치기 전에 반드시 실행하고 통과시킨다:
  ```cmd
  cargo fmt --all
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  ```

## 도메인 규칙 (어기면 결과가 틀어짐)
- **오프셋**: 라벨의 `start/end`는 문자(char) 단위. `CharOffset`/`ByteOffset` newtype을 섞지 않고 변환 함수로만 바꾼다. `regex`·`tokenizers` 결과는 바이트 오프셋이므로 반드시 변환한다 (`tokenizers`는 `encode_char_offsets`).
- **정규화**: NFKC·OCR 치환 후 탐지했으면 대응표로 **원문 위치**로 되돌려 저장한다.
- **규칙 팩 정규식**: `\d`, `\w` 금지 → `[0-9]` 같은 ASCII 클래스. 팩에는 데이터만 넣고 실행 코드는 넣지 않는다 (검증기는 앱에 내장).
- **고유식별정보 항상 가림**: `RRN`·`FRN`·`PASSPORT`·`DRIVER_LICENSE`는 소속(`owner`)과 관계없이 항상 가린다. 규칙 팩 정책으로 끌 수 없고(`pii_core::Label::ALWAYS_MASKED`), 끄려는 팩은 거부한다.
- **합성 비율은 `assets/synth/ratios.toml`에만** 둔다 (템플릿·코드에 숫자로 박지 않음). 근거 없이 정한 값은 `arbitrary = true`.
- **유출 방지 우선**: 애매하면 가린다. SLM이 준 문자열이 청크 안에 여러 번 나오면 모두 가린다. 원문에서 못 찾으면 버린다(환각).
- **PDF 마스킹**: 박스만 덮지 말고 래스터화해서 텍스트 레이어까지 제거한다.
- **로그·감사 로그·에러 메시지에 개인정보 원문 값을 남기지 않는다** (라벨·위치·탐지기만).
- 합성 데이터에서 LLM은 **슬롯 문장만** 만든다. 문서 전체 생성, 값 채우기, 라벨 계산은 Rust 코드가 한다.

## 개선 기록
- 평가(`eval-text`, `run-images`)는 실행마다 `results/runs/<실행ID>/`에 `metrics.json`(지표·신뢰구간·목표 대비 판정)과 `failures.json`(실패 유형별 건수)을 남긴다. 실행할 때 `--note "변경 사항"`을 적는다.
- **평가 후에는 반드시 `pii-mask eval issues`를 돌려** `docs/issues.md`(이슈별 자동 영역)와 `docs/run_log.md`(실행마다 한 줄)를 갱신한다. 목표 미달 지표는 이슈가 자동 등록되므로, 그 이슈의 해석 영역(왜 문제인가·원인·개선안)을 채운다.
- 이슈 형식: 상태 / 자동 영역 / 왜 문제인가 / 원인 / 개선안 / 시도 기록. 자동 영역(`<!-- auto:start -->`~`<!-- auto:end -->`)은 손으로 고치지 않는다.
- **개선을 시도할 때마다** 그 이슈의 시도 기록 표에 날짜·시도·실행 ID(전 → 후)·**전 → 후 수치**·결과를 한 줄 남긴다. 악화·변화 없음도 적는다.
- 해결된 이슈도 지우지 않는다 (상태만 "해결"로).
- 실패 예시 문서(ID·형식)는 **개발용 세트에서만** 적는다. 평가용 세트(`eval_*`, `text_*`)는 건수만 적고 문서를 열어보지 않는다.

## 데이터·개인정보
- **실제 개인정보를 쓰지 않는다.** 모든 데이터는 합성이다. 실제 문서를 작업 폴더에 넣지 않는다.
- `assets/forms/`의 원본 HWP와 `assets/forms/reference/`의 원본 PDF는 수정·삭제하지 않는다. 추출물은 `assets/forms/extracted/`에.
- 대용량은 정해진 곳에만: 모델 `models/`, 생성 데이터 `data/`, 실험 결과 `results/`.
- 클라우드 학습(2차 확장)에는 합성 데이터만 올린다.

## 라이선스
- 크레이트·모델을 추가하기 전에 라이선스를 확인한다. **AGPL 금지** (`mupdf` 크레이트, Ultralytics YOLO 등).
- 기기에 함께 배포하는 모델은 재배포 조건을 확인한다. Kanana는 제3자 제공·판매 시 별도 상업 라이선스 필요 → 비교용으로만.
- 추가한 모델·크레이트는 `THIRD_PARTY_NOTICES.md`에 기록한다.

## 자주 쓰는 명령 (cmd)
```cmd
cargo build --workspace
cargo run -p pii-mask -- forms extract
cargo run -p pii-mask -- forms extract --overlay
cargo run -p pii-mask -- forms extract --input "assets\forms\reference\표준_근로계약서_원본.pdf" --out assets\forms\extracted --dpi 200
cargo run --release -p pii-mask -- synth --mode image --n 20 --seed 41 --out data\dev_s41
cargo run --release -p pii-mask -- synth --mode image --n 100 --seed 42 --out data\eval_s42
cargo run --release -p pii-mask -- synth --mode text --n 1000 --seed 43 --out data\text_s43
cargo run --release -p pii-mask -- data-report --set data\dev_s41 --set data\eval_s42 --set data\text_s43
vendor\llama.cpp\b12202\llama-server.exe -m models\slm\Qwen3.5-4B-Q4_K_M.gguf -ngl 99 -c 8192 --port 8089 --host 127.0.0.1 --jinja -np 1
cargo run --release -p pii-mask -- eval-text --exp e1
cargo run --release -p pii-mask -- eval-text --exp e4
cargo run --release -p pii-mask -- run-images --set data\dev_s41 --out results\e5_dev.json --masked-out data\masked_dev_s41
cargo run --release -p pii-mask -- run-images --set data\eval_s42 --out results\e5_eval.json
cargo run --release -p pii-mask -- final-report
cargo run --release -p pii-mask -- eval issues
cargo run --release -p pii-mask -- eval dashboard
cargo run --release -p pii-mask -- synth --mode text --n 30 --seed 48 --out data\text_holdout_s48 --templates assets\templates_holdout
cargo test -p 크레이트이름
cargo test -p synth --test templates
cargo test -p synth --test clauses
cargo test -p synth --test pdf
where cargo
```
- `forms extract`는 원본 PDF → `assets\forms\extracted\`에 쪽별 PNG(200dpi)·텍스트·글자 위치 JSON과 양식별 텍스트를 쓴다. `--overlay`는 글자 박스를 그린 확인용 PNG(`page_XX.overlay.png`)도 만든다. 텍스트가 없거나 깨진 쪽이 있으면 실패로 끝난다.
- `synth`는 데이터 세트를 만든다 (출력 폴더는 비어 있어야 함). **개발용 `data\dev_s41`(20건)으로만 8~12단계를 개발하고, 평가용 `data\eval_s42`(100건)는 13단계 판정 때만 쓴다 — 개발 중 평가용 이미지를 열어보지 않는다.** `data-report`는 `results\data_report.md`를 만든다.
- `eval-text`(E1 규칙 / E2 +NER / E4 +SLM / E4b 특약 전체 SLM)는 텍스트 세트를, `run-images`는 이미지·PDF 세트를 OCR → 탐지 → 서명란 → 마스킹 → 채점한다 (SLM은 llama-server를 먼저 띄워야 함, `--no-slm`으로 끔). `--masked-out`을 주면 마스킹 결과와 확인용 `_check.png`(정답 박스: 가려짐 초록·유출 빨강·서명란 파랑/주황)를 쓴다 — 평가용 세트에는 쓰지 않는다. `final-report`는 `results\e1·e2·e4·e5_eval.json`으로 `results\report.md`(13단계 판정)를 만든다. `ocr`·`detect`·`mask` 등 단독 서브커맨드는 아직 틀만 있다.
- `cargo test -p synth --test clauses`는 `assets\clauses\*.jsonl` 문장 풀을 검증한다 (자리표시·조사·숫자·트리거 없는 조합). 문장을 고치면 실행. 전체 조합 검토 문서는 `cargo test -p synth --test clauses -- --ignored` → `results\clauses_review_full.md`.
- `cargo test -p synth --test pdf`는 D0 텍스트 레이어 PDF를 pdfium으로 다시 읽어 라벨과 대조한다 (pdfium DLL 필요). 샘플은 `-- --ignored` → `results\d0_sample\`.
- `cargo test -p synth --test templates`는 `assets\templates\*.toml`을 검증하고 원본 PDF 추출 텍스트와 고정 문구를 대조한다. 템플릿을 고치면 반드시 실행 (추출물이 없으면 `forms extract` 먼저).
- 한글·괄호가 든 경로는 따옴표로 감싼다: `dir "assets\forms\표준_근로계약서(7종)(19.6월).hwp"`
