# 2026-10-10 프로젝트 분류 · MAMP 바로 가져오기 (+ 개발 폴더 정리)

## 한 것
- 프로젝트 목록 분류: 언어(전체·PHP·Python·Node) + CMS(WordPress·Gnuboard·Zeroboard·Rhymix/XE·기타). `src/domain/detect.rs` `detect_cms`, `src/ui/projects.rs` `filters()` — 커밋 bcc7d95
- MAMP → 로컬맨 바로 가져오기: `src/domain/mamp.rs` `dump_to_dir`·`import_direct`, `src/ui/transfer.rs` `MampImportDirect` — 커밋 e22e661
- 테스트 68개 통과(새 `detects_cms_kinds` 포함). 둘 다 origin/master 푸시.

## 확인 못 한 것
- 바로 가져오기를 실제 MAMP 데이터로 돌려 보지 않았다(로컬 DB 에 쓰는 작업). 처음엔 DB 1~2개로 시험.
- 분류 버튼 화면은 빌드만 확인, 앱을 띄워 눈으로 보지는 않았다.

## 같은 세션의 로컬맨 밖 작업 (기록용)
- `~/dev/web` 고객 사이트 18개 → `~/dev/clients` 이동, LocalMan `lig`·`tiger` 경로·vhost 수정(백업 `projects.json.bak-before-clients`)
- 개인 라이믹스 8개 아카이브 `~/dev/web/_archive`, 확장 창고 `~/dev/web/rx-extensions`, eond 2.1.38 재구성 `~/dev/web/eond-next`(LocalMan `eond-next`, DB `eond_next`)
- 자세한 위치는 메모리 dev-folders-layout · rhymix-upgrade-pitfalls

## 다음에 할 일
1. eond-next 관리자 로그인 확인(모듈 업데이트 알림) → 이상 없으면 eond ↔ eond-next 전환, LocalMan `rx` 경로·DB 맞추기
2. eond 브랜치 원격 저장소 정하기(eondcom/eond_imin 또는 새 저장소). 지금 origin 은 rhymix 공식이라 push 금지
3. rx-extensions 를 원격에 올리려면 eond_jisung/images(1GB)·manager/frontend/yncare/docs(654MB) 정리 먼저
4. (로컬맨) 사이트 아카이브 기능을 앱에 넣을지 — 이번엔 스크립트로 함

관련 세션: `claude --resume 1faba268-77e3-4f3a-8818-3d4673ca1fdc`
