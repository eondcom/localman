# 다른 프로젝트에서 LocalMan 쓰기 (MCP)

다른 저장소에서 Claude Code 를 쓰다가 "이 폴더를 로컬 사이트로 등록하고 DB 만들어 줘"라고 하면
Claude 가 LocalMan 의 MCP 도구로 처리한다. 이 문서는 그 준비와 말하는 법이다.

## 1. 한 번만 — 등록

```bash
claude mcp add -s user localman -- /Applications/LocalMan.app/Contents/MacOS/LocalMan --mcp
claude mcp get localman        # Scope: User config, Status: ✔ Connected 이면 된다
```

- **`-s user` 를 꼭 붙인다.** 빼면 기본값 local 이라 *명령을 실행한 그 폴더*에서만 도구가 보인다
  — 다른 프로젝트에서 "LocalMan 으로 만들어 줘"라고 해도 Claude 가 도구를 못 찾는 이유가 이것이다.
  이미 local 로 등록했다면: `claude mcp remove localman -s local` 후 위 명령.
- 설정 → 자동화의 [Claude Code 명령 복사]도 같은 명령을 복사한다.
- 리눅스는 실행 파일 경로만 다르다(`which localman`).
- **등록 후 새로 연 Claude Code 세션부터** 도구가 보인다. 이미 열려 있던 세션은 `/mcp` 로 확인하거나 다시 연다.
- LocalMan 앱이 꺼져 있어도 된다 — MCP 는 실행 파일을 따로 띄워 같은 설정 파일(projects.json 등)을 읽고 쓴다.
  화면에 바로 안 보이면 앱의 프로젝트 탭을 한 번 눌러 새로 고친다.

## 2. 이렇게 말하면 된다

| 하고 싶은 일 | 예시 | 쓰이는 도구 |
|---|---|---|
| 지금 폴더를 사이트로 | "이 폴더 LocalMan 에 php 프로젝트로 등록해 줘, 아이디는 shop" | `add_project` |
| DB 만들기 | "LocalMan 으로 shop DB 만들어 줘" | `create_database` |
| 프로젝트 + DB 한 번에 | "LocalMan 으로 이 폴더 프로젝트(shop) 만들고 DB(shop)도 만들어 줘" | `add_project` → `create_database` |
| 설정 파일 속 DB 계정 만들기 | "사이트 설정에 적힌 DB 사용자 LocalMan 으로 만들어 줘" | `ensure_site_db_users` |
| 서버 켜기 | "LocalMan 에서 Apache 랑 DB 켜 줘" | `start_service` |
| 목록 보기 | "LocalMan 사이트 목록 / DB 목록 보여 줘" | `list_projects` · `list_databases` |
| 백업 | "shop DB 백업해 줘" | `backup_database` |
| 실서버에서 받기 | "shop 사이트 서버에서 파일이랑 DB 받아 와" | `pull_from_server` |
| eond.com 배포 | "eondctl 로 status / dry-run 해 줘" | `eondctl` |

"LocalMan" 이라는 말을 넣으면 Claude 가 이 도구를 고른다.

## 3. 도구별 알아 둘 것

- **add_project** `{id, path, type?, name?, start_command?}`
  - `id` 는 소문자·숫자·`-` — 주소가 `http://<id>.localhost` 가 된다. 이미 있는 id 면 실패.
  - `path` 는 **이미 있는 절대 경로 폴더**. 폴더를 새로 만들지는 않는다(먼저 만든다).
  - `type`: `php`(기본, Apache 가 바로 서빙) · `python` · `nextjs`(개발 서버를 Apache 가 프록시, 포트·실행 명령 자동).
  - DB 를 프로젝트에 연결하지는 않는다 — DB 는 `create_database` 로 따로.
- **create_database** `{name, engine?}` — 빈 DB(utf8mb4). `engine`: `mariadb`(기본, MySQL 포함) · `postgresql`.
  LocalMan 에 저장된 DB 접속 정보(데이터베이스 탭)로 만든다 — 저장된 게 없으면 실패하니 앱에서 한 번 접속해 둔다.
  DB 사용자는 만들지 않는다. 사이트 설정 파일(config.php·wp-config.php·.env)에 계정을 적어 두고 `ensure_site_db_users` 를 부르면 그 계정을 만들고 권한을 준다(기존 비밀번호는 안 바꾼다).
- **start_project / stop_project** — python·nextjs 사이트의 개발 서버만. php 는 `start_service apache`.
- **pull_from_server** — 앱의 프로젝트 → ⋯ → 서버 연결에 서버 정보가 저장된 사이트만. 로컬 파일은 지우지 않고, DB 를 받기 전 로컬 DB 를 백업한다.

## 4. 열지 않은 것

DB·프로젝트 삭제, 서버로 올리기(배포), 비밀번호 바꾸기는 MCP·API 로 열지 않았다 — 앱에서 직접 한다.

## 5. 안 될 때

| 증상 | 확인 |
|---|---|
| Claude 가 "LocalMan 도구가 없다"고 한다 | `claude mcp get localman` 의 Scope 가 *Local* 이면 1번대로 user 로 다시 등록. 등록 후 세션을 새로 연다 |
| `Status: ✗ Failed` | 경로 확인(`ls /Applications/LocalMan.app/Contents/MacOS/LocalMan`), Homebrew 로 다시 설치했으면 경로가 같은지 |
| create_database 실패 | LocalMan 데이터베이스 탭에서 접속 정보 저장, DB 서버 켜짐(`service_status`) |
| 사이트 주소가 안 열림 | Apache 켜짐, 맥 Safari 면 `sudo killall mDNSResponder` 후 다시 |

## HTTP API (스크립트에서)

앱의 설정 → 자동화 → HTTP API 를 켜면 앱이 떠 있는 동안 `127.0.0.1:47810` 에서 같은 도구를 부를 수 있다(토큰 필요).

```bash
curl -H "Authorization: Bearer <토큰>" http://127.0.0.1:47810/api/tools
curl -H "Authorization: Bearer <토큰>" -X POST http://127.0.0.1:47810/api/tools/create_database -d '{"name":"shop"}'
```

eondctl 은 이 API 로 LocalMan 의 로컬 서버를 켜고 끄고, LocalMan 은 eondctl 의 API(`127.0.0.1:47811`)나 CLI 로 배포를 부른다 — README 의 "eondctl 연동".
