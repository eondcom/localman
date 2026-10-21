# MCP 등록 — OS 공통 `--mcp-install`

## 상태
- 브랜치 `worktree-mcp-install` (커밋 e184d17) 푸시 완료, **master 머지 전**.
- Linux 에서 `--mcp-install` 실행 → `claude mcp get localman` 이 `✔ Connected` 로 확인함. macOS 는 실행 못 해서 미확인.

## 배경
- README·docs/mcp-guide.md 의 등록 명령이 macOS 경로(`/Applications/LocalMan.app/...`)로 고정돼 있어 Linux 에서 `ENOENT` 로 실패했다.
- `~/.local/share/localman/localman` 은 10/02 구버전이라 `--mcp` 에 응답하지 않는다. 등록 대상은 `cargo build --release` 결과물.

## 구조
| 경로 | 내용 |
|---|---|
| `src/mcp.rs` | `install()` — `current_exe()` 로 기존 등록 삭제 후 `claude mcp add -s user localman -- <exe> --mcp` |
| `src/main.rs` | `--mcp-install` 분기 (`--mcp` 보다 먼저) |
| `README.md`, `docs/mcp-guide.md` | `--mcp-install` 기준으로 안내, 문제 해결 표에 OS 경로 불일치 안내 |

## 다음에 할 일
1. `worktree-mcp-install` 을 master 에 머지 (PR: https://github.com/eondcom/localman/pull/new/worktree-mcp-install).
2. 머지 후 `cargo build --release` 한 바이너리로 `--mcp-install` 다시 실행해 등록 경로 갱신 (지금은 `/home/dell/dev/localman/target/release/localman`, 이 바이너리는 `--mcp-install` 미포함 빌드).
3. macOS 에서 `--mcp-install` 동작 확인.
4. 미커밋 `assets/NanumGothic.ttf`(4.6MB) — `master` 의 `src/` 가 참조하지 않음. 커밋/ignore/삭제 결정 필요.
5. 오래된 워크트리 `macos-plan`(잠김)·`toast-save-errors` 정리 여부 결정.

## 세션 로그
- **2026-10-18**: git 상태 확인 → MCP/API 로컬 존재 확인 → macOS 경로 등록 실패 원인 진단·Linux 경로로 재등록 → `--mcp-install` 구현·문서 수정·푸시.
