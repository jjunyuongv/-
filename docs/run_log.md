# 실행 기록

`pii-mask eval issues`가 실행마다 한 줄씩 더한다 (`results/runs/<실행 ID>/`의 metrics.json·failures.json).

| 실행 ID | 날짜 | 데이터 (시드) | 변경 사항 | 주요 지표 |
|---|---|---|---|---|
| `20261108-170000-e1` | 2026-11-08 | 평가용 `data/text_s43` (시드 43, 1000건) | 규칙 팩 첫 버전 (사후 등록) | structured_recall 95.4%✗, name_address_recall 100.0%, fp_rate 0.2% |
| `20261108-170100-e2` | 2026-11-08 | 평가용 `data/text_s43` (시드 43, 1000건) | 규칙 팩 첫 버전 + NER 이름·주소 (사후 등록) | structured_recall 95.4%✗, name_address_recall 100.0%, fp_rate 0.8% |
| `20261108-173000-e1` | 2026-11-08 | 평가용 `data/text_s43` (시드 43, 1000건) | 점(.)·공백 구분자, 새마을금고 4-2-6-1 형식 보강 — 분석은 개발용 텍스트 시드 44 (사후 등록) | structured_recall 100.0%, name_address_recall 100.0%, fp_rate 0.2% |
| `20261108-173100-e2` | 2026-11-08 | 평가용 `data/text_s43` (시드 43, 1000건) | 같은 보강 (사후 등록) | structured_recall 100.0%, name_address_recall 100.0%, fp_rate 0.8% |
| `20261108-180000-e4` | 2026-11-08 | 평가용 `data/text_s43` (시드 43, 1000건) | SLM 트리거 방식 (사후 등록) | structured_recall 100.0%, name_address_recall 100.0%, sensitive_recall 62.0%✗, fp_rate 0.8% |
| `20261108-183000-e5` | 2026-11-08 | 개발용 `data/dev_s41` (시드 41, 20건) | 개발용 세트 확인 — 서명란 붉은 덩어리 연결 성분, 앵커 조각, 빈 값 키 (사후 등록) | leak_rate_d02 6.0%✗, structured_recall_d12 100.0%, signature_coverage 78.9%✗, fp_rate 4.4% |
| `20261108-190000-e5` | 2026-11-08 | 평가용 `data/eval_s42` (시드 42, 100건) | E5 첫 실행 (사후 등록) | leak_rate_d02 13.5%✗, structured_recall_d12 90.0%✗, signature_coverage 59.4%✗, fp_rate 5.3%✗ |
| `20261108-191500-e5` | 2026-11-08 | 개발용 `data/dev2_s45` (시드 45, 40건) | 개발용 2차 세트 — 쪽 기울기 행 묶기·좌우 여백 30% (사후 등록) | leak_rate_d02 7.6%✗, structured_recall_d12 98.0%, signature_coverage 66.7%✗, fp_rate 2.2% |
| `20261108-193000-e5` | 2026-11-08 | 평가용 `data/eval_s42` (시드 42, 100건) | 쪽 기울기 행 묶기·마스크 좌우 여백 30% (사후 등록) | leak_rate_d02 11.4%✗, structured_recall_d12 91.7%✗, signature_coverage 59.4%✗, fp_rate 6.1%✗ |
| `20261108-200000-e4b` | 2026-11-08 | 평가용 `data/text_s43` (시드 43, 300건) | 추가 실험: 특약 구역 전체 줄을 SLM에 (앞 300건, 사후 등록) | structured_recall 100.0%, name_address_recall 100.0%, sensitive_recall 96.2%, fp_rate 0.8% |
| `20261108-201500-e4` | 2026-11-08 | 평가용 `data/text_s43` (시드 43, 300건) | E4b 비교용 같은 300건 (사후 등록) | structured_recall 100.0%, name_address_recall 100.0%, sensitive_recall 66.7%✗, fp_rate 0.8% |
| `20261108-203000-e1` | 2026-11-08 | 평가용 `data/text_s43` (시드 43, 1000건) | 최종 팩으로 다시 실행 — 숫자 변화 없음 (사후 등록) | structured_recall 100.0%, name_address_recall 100.0%, fp_rate 0.2% |
| `20261108-203100-e2` | 2026-11-08 | 평가용 `data/text_s43` (시드 43, 1000건) | 최종 팩으로 다시 실행 — 숫자 변화 없음 (사후 등록) | structured_recall 100.0%, name_address_recall 100.0%, fp_rate 0.8% |
| `20261108-214748-e5` | 2026-11-08 | 개발용 `data/dev_s41` (시드 41, 20건) | 필드 영역 마스킹 + 서명란 블록(':' 뒤부터) + 서명란 실패 진단 | leak_rate_d02 1.5%✗, structured_recall_d12 100.0%, signature_coverage 78.9%✗, fp_rate 11.1%✗ |
| `20261108-215006-e5` | 2026-11-08 | 개발용 `data/dev2_s45` (시드 45, 40건) | 필드 영역 마스킹 + 서명란 블록(':' 뒤부터) + 서명란 실패 진단 | leak_rate_d02 3.8%✗, structured_recall_d12 98.0%, signature_coverage 74.5%✗, fp_rate 4.5% |
| `20261108-215340-e5` | 2026-11-08 | 개발용 `data/dev_s41` (시드 41, 20건) | 서명란: 앵커가 가려진 서명 줄은 줄 끝을 앵커로 | leak_rate_d02 1.5%✗, structured_recall_d12 100.0%, signature_coverage 84.2%✗, fp_rate 11.1%✗ |
| `20261108-215724-e5` | 2026-11-08 | 개발용 `data/dev2_s45` (시드 45, 40건) | 서명란: 앵커가 가려진 서명 줄은 줄 끝을 앵커로 | leak_rate_d02 3.8%✗, structured_recall_d12 98.0%, signature_coverage 86.3%✗, fp_rate 4.5% |
| `20261108-220440-e5` | 2026-11-08 | 개발용 `data/dev2_s45` (시드 45, 40건) | 이미지 경로 E4b (특약 구역 전체 SLM) [SLM: 특약 구역 전체] | leak_rate_d02 2.7%✗, structured_recall_d12 98.0%, signature_coverage 86.3%✗, fp_rate 4.5% |
| `20261108-221900-e5` | 2026-11-08 | 개발용 `data/dev_s41` (시드 41, 20건) | 서명 줄 대체 앵커는 날짜 줄 아래에만 (과잉 마스킹 줄임) + 이미지 E4b [SLM: 특약 구역 전체] | leak_rate_d02 0.8%, structured_recall_d12 100.0%, signature_coverage 84.2%✗, fp_rate 11.1%✗ |
| `20261108-222724-e5` | 2026-11-08 | 개발용 `data/dev2_s45` (시드 45, 40건) | 서명 줄 대체 앵커는 날짜 줄 아래에만 (과잉 마스킹 줄임) + 이미지 E4b [SLM: 특약 구역 전체] | leak_rate_d02 2.7%✗, structured_recall_d12 98.0%, signature_coverage 86.3%✗, fp_rate 4.5% |
| `20261108-224647-e5` | 2026-11-08 | 개발용 `data/dev_s41` (시드 41, 20건) | 기울어진 줄: 글자 박스 세로 범위를 줄 4점에 맞추고 탐지·필드 사각형을 묶음으로 나눔 (오탐 줄이기) + 이미지 E4b [SLM: 특약 구역 전체] | leak_rate_d02 0.8%, structured_recall_d12 100.0%, signature_coverage 84.2%✗, fp_rate 4.4% |
| `20261108-225103-e5` | 2026-11-08 | 개발용 `data/dev2_s45` (시드 45, 40건) | 기울어진 줄: 글자 박스 세로 범위를 줄 4점에 맞추고 탐지·필드 사각형을 묶음으로 나눔 (오탐 줄이기) + 이미지 E4b [SLM: 특약 구역 전체] | leak_rate_d02 2.7%✗, structured_recall_d12 98.0%, signature_coverage 86.3%✗, fp_rate 2.2% |
| `20261108-230943-e5` | 2026-11-08 | 평가용 `data/eval2_s46` (시드 46, 100건) | 새 시드 평가 세트: 필드 영역·서명 대체 앵커(날짜 아래)·기울어진 줄 사각형 보정·이미지 E4b [SLM: 특약 구역 전체] | leak_rate_d02 4.1%✗, structured_recall_d12 92.7%✗, signature_coverage 88.4%✗, fp_rate 5.2%✗ |
| `20261109-003827-e4b` | 2026-11-09 | 평가용 `data/text_s43` (시드 43, 1000건) | E4b 전체 1000건 (특약 구역 전체 SLM) | structured_recall 100.0%, name_address_recall 100.0%, sensitive_recall 95.8%, fp_rate 0.8% |
| `20261109-021237-h_e1` | 2026-11-09 | 평가용 `data/text_holdout_s48` (시드 48, 30건) | 처음 보는 서식 시험 (서식 3종 30건, 탐지 코드 수정 없이 한 번) | structured_recall 100.0%, name_address_recall 47.1%✗, fp_rate 9.5%✗ |
| `20261109-021239-h_e2` | 2026-11-09 | 평가용 `data/text_holdout_s48` (시드 48, 30건) | 처음 보는 서식 시험 (서식 3종 30건, 탐지 코드 수정 없이 한 번) | structured_recall 100.0%, name_address_recall 100.0%, fp_rate 14.3%✗ |
| `20261109-021624-h_e4b` | 2026-11-09 | 평가용 `data/text_holdout_s48` (시드 48, 30건) | 처음 보는 서식 시험 (서식 3종 30건, 탐지 코드 수정 없이 한 번) | structured_recall 100.0%, name_address_recall 100.0%, sensitive_recall 5.3%✗, fp_rate 14.3%✗ |
