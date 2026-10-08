//! 디자인 토큰과 공용 컴포넌트 — EOND UI App 0.2 (맥북 팬 관리와 같은 체계).
//!
//! 원칙: 테두리 대신 밝기(바탕 → c1 카드 → c2 겹침 → c3 선택)로 층을 나누고, 색은 토큰으로만 쓴다.
//! 색 값은 mac-fan-control/App/Design/EUTheme.swift 와 같다 (HeroUI 팔레트).
//! iced 가 화면을 직접 그리고 글꼴(Pretendard JP)·아이콘(Lucide)을 앱에 넣으므로 맥·리눅스가 똑같이 보인다.

use iced::widget::{button, checkbox, column, container, pick_list, progress_bar, row, text, text_input, toggler, Space};
use iced::{font, Background, Border, Color, Element, Font, Length, Shadow, Theme, Vector};
use std::sync::atomic::{AtomicBool, Ordering};

// ── 글꼴 ───────────────────────────────────────────────────────────────

/// 글꼴은 Pretendard JP 하나로 쓴다 — 한글·가나·한자·영문이 모두 들어 있다.
/// (한국어판 Pretendard 에는 한자가 없어 일본어 화면의 한자가 네모로 깨졌다. iced 는 시스템 글꼴로
///  대신 그려 주지 않으므로 필요한 글자가 다 든 글꼴 하나를 쓰는 게 가장 확실하다)
pub const FONT_FILES: [&[u8]; 5] = [
    include_bytes!("../../assets/fonts/PretendardJP-Regular.otf"),
    include_bytes!("../../assets/fonts/PretendardJP-Medium.otf"),
    include_bytes!("../../assets/fonts/PretendardJP-SemiBold.otf"),
    include_bytes!("../../assets/fonts/PretendardJP-Bold.otf"),
    include_bytes!("../../assets/fonts/lucide.ttf"),
];

pub const REGULAR: Font = Font::with_name("Pretendard JP");
pub const MEDIUM: Font = Font { weight: font::Weight::Medium, ..REGULAR };
pub const SEMIBOLD: Font = Font { weight: font::Weight::Semibold, ..REGULAR };
pub const BOLD: Font = Font { weight: font::Weight::Bold, ..REGULAR };
const ICON_FONT: Font = Font::with_name("lucide");

// ── 색 ────────────────────────────────────────────────────────────────

const fn hex(v: u32) -> Color {
    Color::from_rgb(
        ((v >> 16) & 0xFF) as f32 / 255.0,
        ((v >> 8) & 0xFF) as f32 / 255.0,
        (v & 0xFF) as f32 / 255.0,
    )
}

const fn hexa(v: u32, a: f32) -> Color {
    let c = hex(v);
    Color { a, ..c }
}

#[derive(Debug, Clone, Copy)]
pub struct Pal {
    pub app_bg: Color,
    pub chrome: Color,
    pub c1: Color,
    pub c2: Color,
    pub c3: Color,
    pub c4: Color,
    pub fg: Color,
    pub fg2: Color,
    pub fg3: Color,
    pub fg4: Color,
    pub line: Color,
    pub hover: Color,
    pub shadow: Color,
    pub primary: Color,
    pub primary_flat: Color,
    pub primary_fg: Color,
    pub on_primary: Color,
    pub success: Color,
    pub success_flat: Color,
    pub success_fg: Color,
    pub warning: Color,
    pub warning_flat: Color,
    pub warning_fg: Color,
    pub danger: Color,
    pub danger_flat: Color,
    pub danger_fg: Color,
}

pub const DARK: Pal = Pal {
    app_bg: hex(0x000000),
    chrome: hex(0x000000),
    c1: hex(0x18181B),
    c2: hex(0x27272A),
    c3: hex(0x3F3F46),
    c4: hex(0x52525B),
    fg: hex(0xECEDEE),
    fg2: hex(0xD4D4D8),
    fg3: hex(0xA1A1AA),
    fg4: hex(0x71717A),
    line: hexa(0xFFFFFF, 0.06),
    hover: hexa(0xFFFFFF, 0.05),
    shadow: hexa(0x000000, 0.5),
    primary: hex(0x006FEE),
    primary_flat: hexa(0x006FEE, 0.18),
    primary_fg: hex(0x338EF7),
    on_primary: hex(0xFFFFFF),
    success: hex(0x17C964),
    success_flat: hexa(0x17C964, 0.14),
    success_fg: hex(0x45D483),
    warning: hex(0xF5A524),
    warning_flat: hexa(0xF5A524, 0.16),
    warning_fg: hex(0xF7B750),
    danger: hex(0xF31260),
    danger_flat: hexa(0xF31260, 0.16),
    danger_fg: hex(0xF54180),
};

pub const LIGHT: Pal = Pal {
    app_bg: hex(0xF4F4F5),
    chrome: hex(0xFFFFFF),
    c1: hex(0xFFFFFF),
    c2: hex(0xF4F4F5),
    c3: hex(0xE4E4E7),
    c4: hex(0xD4D4D8),
    fg: hex(0x11181C),
    fg2: hex(0x3F3F46),
    fg3: hex(0x71717A),
    fg4: hex(0xA1A1AA),
    line: hexa(0x000000, 0.06),
    hover: hexa(0x000000, 0.04),
    shadow: hexa(0x000000, 0.06),
    primary: hex(0x006FEE),
    primary_flat: hex(0xE6F1FE),
    primary_fg: hex(0x005BC4),
    on_primary: hex(0xFFFFFF),
    success: hex(0x17C964),
    success_flat: hex(0xE8FAF0),
    success_fg: hex(0x0E793C),
    warning: hex(0xF5A524),
    warning_flat: hex(0xFEF6E6),
    warning_fg: hex(0x936316),
    danger: hex(0xF31260),
    danger_flat: hex(0xFEE7EF),
    danger_fg: hex(0xC20E4D),
};

static IS_DARK: AtomicBool = AtomicBool::new(true);

pub fn set_dark(dark: bool) {
    IS_DARK.store(dark, Ordering::Relaxed);
}

pub fn is_dark() -> bool {
    IS_DARK.load(Ordering::Relaxed)
}

/// 지금 테마의 색
pub fn p() -> &'static Pal {
    if is_dark() { &DARK } else { &LIGHT }
}

/// iced 기본 위젯(스크롤바·선택 목록 등)도 같은 색을 쓰게 하는 테마
pub fn iced_theme() -> Theme {
    let c = p();
    Theme::custom(
        if is_dark() { "EOND Dark" } else { "EOND Light" }.to_string(),
        iced::theme::Palette {
            background: c.app_bg,
            text: c.fg,
            primary: c.primary,
            success: c.success,
            danger: c.danger,
        },
    )
}

// ── 모서리 ─────────────────────────────────────────────────────────────

pub const R_CHIP: f32 = 7.0;
pub const R_ROW: f32 = 9.0;
pub const R_BTN: f32 = 10.0;
pub const R_CARD: f32 = 14.0;

fn radius(r: f32) -> Border {
    Border { radius: r.into(), ..Default::default() }
}

// ── 아이콘 (Lucide) ────────────────────────────────────────────────────

/// 쓸 수 있는 아이콘 목록 (지금 화면에서 안 쓰는 것도 둔다)
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub enum Icon {
    Server,
    /// 앱 로고 (집 + 플러그)
    HousePlug,
    Heart,
    QrCode,
    Folder,
    FolderOpen,
    Database,
    ArrowLeftRight,
    Refresh,
    Play,
    Square,
    Settings,
    ShieldCheck,
    Lock,
    Send,
    Download,
    Upload,
    Ellipsis,
    FileText,
    Pencil,
    Trash,
    Globe,
    Wifi,
    Check,
    X,
    HardDrive,
    History,
    Search,
    Plus,
    Terminal,
    Key,
    Package,
    ExternalLink,
    Copy,
    Monitor,
    Laptop,
    Layers,
    Sun,
    Moon,
    Zap,
}

impl Icon {
    fn ch(self) -> char {
        let cp = match self {
            Icon::Server => 0xe156,
            Icon::HousePlug => 0xe5f4,
            Icon::Heart => 0xe0f5,
            Icon::QrCode => 0xe1de,
            Icon::Folder => 0xe0dc,
            Icon::FolderOpen => 0xe246,
            Icon::Database => 0xe0b1,
            Icon::ArrowLeftRight => 0xe249,
            Icon::Refresh => 0xe148,
            Icon::Play => 0xe13f,
            Icon::Square => 0xe16a,
            Icon::Settings => 0xe157,
            Icon::ShieldCheck => 0xe1fe,
            Icon::Lock => 0xe10e,
            Icon::Send => 0xe155,
            Icon::Download => 0xe0b6,
            Icon::Upload => 0xe19d,
            Icon::Ellipsis => 0xe0ba,
            Icon::FileText => 0xe0d0,
            Icon::Pencil => 0xe1f8,
            Icon::Trash => 0xe18d,
            Icon::Globe => 0xe0eb,
            Icon::Wifi => 0xe1ad,
            Icon::Check => 0xe070,
            Icon::X => 0xe1b1,
            Icon::HardDrive => 0xe0f0,
            Icon::History => 0xe1f4,
            Icon::Search => 0xe154,
            Icon::Plus => 0xe140,
            Icon::Terminal => 0xe184,
            Icon::Key => 0xe4a7,
            Icon::Package => 0xe12c,
            Icon::ExternalLink => 0xe0bd,
            Icon::Copy => 0xe0a2,
            Icon::Monitor => 0xe120,
            Icon::Laptop => 0xe1cc,
            Icon::Layers => 0xe52d,
            Icon::Sun => 0xe17b,
            Icon::Moon => 0xe121,
            Icon::Zap => 0xe1b3,
        };
        char::from_u32(cp).unwrap_or(' ')
    }
}

pub fn icon<'a, M: 'a>(i: Icon, size: f32, color: Color) -> Element<'a, M> {
    text(i.ch()).font(ICON_FONT).size(size).color(color).into()
}

// ── 글자 ───────────────────────────────────────────────────────────────

/// 페이지 제목 + 설명 + 오른쪽 동작
pub fn page_header<'a, M: 'a>(title: &'a str, subtitle: &'a str, actions: Option<Element<'a, M>>) -> Element<'a, M> {
    let left = column![
        text(title).size(26).font(BOLD).color(p().fg),
        text(subtitle).size(13).color(p().fg3),
    ]
    .spacing(4)
    .width(Length::Fill);
    let mut r = row![left].align_y(iced::Alignment::Center);
    if let Some(a) = actions {
        r = r.push(a);
    }
    r.into()
}

/// 카드 묶음 위의 작은 제목 ("화면", "실행" 같은)
pub fn section_label<'a, M: 'a>(label: &'a str) -> Element<'a, M> {
    container(text(label).size(12).font(SEMIBOLD).color(p().fg3))
        .padding(iced::Padding { top: 6.0, bottom: 6.0, left: 4.0, right: 0.0 })
        .into()
}

pub fn title<'a>(s: impl text::IntoFragment<'a>) -> text::Text<'a, Theme> {
    text(s).size(15).font(SEMIBOLD).color(p().fg)
}

pub fn body<'a>(s: impl text::IntoFragment<'a>) -> text::Text<'a, Theme> {
    text(s).size(13).color(p().fg)
}

pub fn muted<'a>(s: impl text::IntoFragment<'a>) -> text::Text<'a, Theme> {
    text(s).size(12).color(p().fg3)
}

// ── 그릇 ───────────────────────────────────────────────────────────────

/// 카드: 바탕보다 한 단계 밝은 c1, 테두리 없음
pub fn card<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> Element<'a, M> {
    container(content)
        .padding(18)
        .width(Length::Fill)
        .style(|_| container::Style {
            background: Some(Background::Color(p().c1)),
            border: radius(R_CARD),
            shadow: Shadow { color: p().shadow, offset: Vector::new(0.0, 1.0), blur_radius: if is_dark() { 0.0 } else { 3.0 } },
            ..Default::default()
        })
        .into()
}

/// 설정 화면처럼 한 카드 안에 줄을 묶고 줄 사이에 얇은 선을 넣는다.
pub fn group<'a, M: 'a>(rows: Vec<Element<'a, M>>) -> Element<'a, M> {
    let n = rows.len();
    let mut col = column![];
    for (i, r) in rows.into_iter().enumerate() {
        col = col.push(container(r).padding([12, 18]).width(Length::Fill));
        if i + 1 < n {
            col = col.push(divider());
        }
    }
    container(col)
        .width(Length::Fill)
        .style(|_| container::Style {
            background: Some(Background::Color(p().c1)),
            border: radius(R_CARD),
            shadow: Shadow { color: p().shadow, offset: Vector::new(0.0, 1.0), blur_radius: if is_dark() { 0.0 } else { 3.0 } },
            ..Default::default()
        })
        .into()
}

/// 세로 구분선 (사이드바와 본문 사이)
pub fn vdivider<'a, M: 'a>() -> Element<'a, M> {
    container(Space::with_width(1))
        .width(1)
        .height(Length::Fill)
        .style(|_| container::Style { background: Some(Background::Color(p().line)), ..Default::default() })
        .into()
}

pub fn divider<'a, M: 'a>() -> Element<'a, M> {
    container(Space::with_height(1))
        .width(Length::Fill)
        .height(1)
        .style(|_| container::Style { background: Some(Background::Color(p().line)), ..Default::default() })
        .into()
}

/// 설정 줄: 왼쪽 제목(+설명), 오른쪽 조작
pub fn setting_row<'a, M: 'a>(label: &'a str, help: Option<&'a str>, control: Element<'a, M>) -> Element<'a, M> {
    let mut left = column![body(label)].spacing(2).width(Length::Fill);
    if let Some(h) = help {
        left = left.push(muted(h));
    }
    row![left, control].spacing(16).align_y(iced::Alignment::Center).into()
}

/// 겹친 면(c2) — 카드 안의 부속 영역
pub fn inset<'a, M: 'a>(content: impl Into<Element<'a, M>>) -> Element<'a, M> {
    container(content)
        .padding(12)
        .width(Length::Fill)
        .style(|_| container::Style { background: Some(Background::Color(p().c2)), border: radius(R_ROW), ..Default::default() })
        .into()
}

// ── 칩·점 ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tone {
    Neutral,
    Primary,
    Success,
    Warning,
    Danger,
}

fn tone_colors(t: Tone) -> (Color, Color) {
    let c = p();
    match t {
        Tone::Neutral => (c.c2, c.fg2),
        Tone::Primary => (c.primary_flat, c.primary_fg),
        Tone::Success => (c.success_flat, c.success_fg),
        Tone::Warning => (c.warning_flat, c.warning_fg),
        Tone::Danger => (c.danger_flat, c.danger_fg),
    }
}

pub fn tone_dot_color(t: Tone) -> Color {
    let c = p();
    match t {
        Tone::Neutral => c.fg4,
        Tone::Primary => c.primary,
        Tone::Success => c.success,
        Tone::Warning => c.warning,
        Tone::Danger => c.danger,
    }
}

pub fn chip<'a, M: 'a>(label: impl text::IntoFragment<'a>, t: Tone) -> Element<'a, M> {
    let (bg, fg) = tone_colors(t);
    container(text(label).size(11).font(SEMIBOLD).color(fg))
        .padding([3, 8])
        .style(move |_| container::Style { background: Some(Background::Color(bg)), border: radius(R_CHIP), ..Default::default() })
        .into()
}

pub fn dot<'a, M: 'a>(t: Tone) -> Element<'a, M> {
    let color = tone_dot_color(t);
    container(Space::with_width(7))
        .width(7)
        .height(7)
        .style(move |_| container::Style { background: Some(Background::Color(color)), border: radius(4.0), ..Default::default() })
        .into()
}

/// 점 + 긴 글 (진행 상태처럼 길어질 수 있는 문장). 남은 너비 안에서 줄바꿈한다.
pub fn status_line<'a, M: 'a>(label: impl text::IntoFragment<'a>, t: Tone) -> Element<'a, M> {
    let (_, fg) = tone_colors(t);
    let fg = if t == Tone::Neutral { p().fg3 } else { fg };
    row![column![Space::with_height(6), dot(t)], text(label).size(12).font(MEDIUM).color(fg).width(Length::Fill)]
        .spacing(8)
        .into()
}

/// 점 + 글자 상태 표시 ("실행 중")
pub fn status<'a, M: 'a>(label: impl text::IntoFragment<'a>, t: Tone) -> Element<'a, M> {
    let (_, fg) = tone_colors(t);
    let fg = if t == Tone::Neutral { p().fg3 } else { fg };
    row![dot(t), text(label).size(12).font(MEDIUM).color(fg)].spacing(6).align_y(iced::Alignment::Center).into()
}

/// "✓ …" / "✗ …" / "· …" 결과 줄 — 기호 대신 색 점으로 그린다.
pub fn result_line<'a, M: 'a>(line: impl AsRef<str>) -> Element<'a, M> {
    let line = line.as_ref();
    let (t, rest) = if let Some(r) = line.strip_prefix('✓') {
        (Tone::Success, r)
    } else if let Some(r) = line.strip_prefix('✗') {
        (Tone::Danger, r)
    } else if let Some(r) = line.strip_prefix('·') {
        (Tone::Neutral, r)
    } else {
        (Tone::Neutral, line)
    };
    let fg = if t == Tone::Neutral { p().fg3 } else { p().fg2 };
    // 글자에 남은 너비를 줘야 긴 경로·오류가 카드 밖으로 넘치지 않고 줄바꿈된다
    row![column![Space::with_height(6), dot(t)], text(rest.trim_start().to_string()).size(12).color(fg).width(Length::Fill)]
        .spacing(8)
        .into()
}

/// 결과·오류 줄 묶음 + [로그 복사] — 오류를 그대로 붙여 물어볼 수 있게
pub fn log_block<'a, M: Clone + 'a>(lines: Vec<String>, copy: impl Fn(String) -> M) -> Element<'a, M> {
    let all = lines.join("\n");
    log_block_full(lines, all, copy)
}

/// 화면에는 shown 만, [로그 복사]는 full 을
pub fn log_block_full<'a, M: Clone + 'a>(lines: Vec<String>, all: String, copy: impl Fn(String) -> M) -> Element<'a, M> {
    let col = lines.into_iter().fold(column![].spacing(4), |c, l| c.push(result_line(l)));
    column![
        col,
        row![Space::with_width(Length::Fill), btn(crate::i18n::tr("로그 복사"), Some(Icon::Copy), Kind::Ghost).on_press(copy(all))],
    ]
    .spacing(2)
    .into()
}

// ── 버튼 ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    /// 메인 색 채움 — 화면의 주 동작
    Primary,
    /// c2 면 — 카드 안의 보통 동작
    Flat,
    /// c1 면 + 그림자 — 바탕(페이지) 위의 보통 동작 (제목 옆 [새로고침] 같은)
    Surface,
    /// 배경 없음 — 덜 중요한 동작
    Ghost,
    /// 빨간 면 — 삭제·중지
    Danger,
    /// 초록 면 — 시작
    Success,
}

fn button_style(kind: Kind, status: button::Status) -> button::Style {
    let c = p();
    let (bg, fg) = match kind {
        Kind::Primary => (c.primary, c.on_primary),
        Kind::Flat => (c.c2, c.fg),
        Kind::Surface => (c.c1, c.fg),
        Kind::Ghost => (Color::TRANSPARENT, c.fg2),
        Kind::Danger => (c.danger_flat, c.danger_fg),
        Kind::Success => (c.success_flat, c.success_fg),
    };
    let bg = match status {
        button::Status::Hovered | button::Status::Pressed => match kind {
            Kind::Primary => Color { a: 0.88, ..bg },
            Kind::Flat => c.c3,
            Kind::Surface => c.c2,
            Kind::Ghost => c.hover,
            Kind::Danger | Kind::Success => Color { a: (bg.a + 0.08).min(1.0), ..bg },
        },
        _ => bg,
    };
    let disabled = status == button::Status::Disabled;
    button::Style {
        background: Some(Background::Color(if disabled { Color { a: bg.a * 0.5, ..bg } } else { bg })),
        text_color: if disabled { c.fg4 } else { fg },
        border: if kind == Kind::Surface {
            Border { radius: R_BTN.into(), width: 1.0, color: c.line }
        } else {
            radius(R_BTN)
        },
        shadow: if kind == Kind::Surface && !is_dark() {
            Shadow { color: hexa(0x000000, 0.05), offset: Vector::new(0.0, 1.0), blur_radius: 2.0 }
        } else {
            Shadow::default()
        },
    }
}

/// 버튼. on_press 는 호출한 쪽에서 붙인다 (없으면 비활성으로 그려진다).
pub fn btn<'a, M: Clone + 'a>(label: impl text::IntoFragment<'a>, ic: Option<Icon>, kind: Kind) -> button::Button<'a, M> {
    let c = p();
    let fg = match kind {
        Kind::Primary => c.on_primary,
        Kind::Flat | Kind::Surface => c.fg,
        Kind::Ghost => c.fg2,
        Kind::Danger => c.danger_fg,
        Kind::Success => c.success_fg,
    };
    let content: Element<'a, M> = match ic {
        Some(i) => row![icon(i, 13.0, fg), text(label).size(13).font(MEDIUM)]
            .spacing(6)
            .align_y(iced::Alignment::Center)
            .into(),
        None => text(label).size(13).font(MEDIUM).into(),
    };
    button(content).padding([7, 14]).style(move |_, s| button_style(kind, s))
}

/// 칩 모양의 작은 버튼. 글자를 소유하므로 목록에서 만들어지는 버튼(저장된 연결·권한 등)에 쓴다.
pub fn chip_btn<'a, M: Clone + 'a>(label: String, kind: Kind) -> button::Button<'a, M> {
    button(text(label).size(12).font(MEDIUM)).padding([5, 10]).style(move |_, s| {
        let mut st = button_style(kind, s);
        st.border.radius = R_CHIP.into();
        st
    })
}

/// 아이콘만 있는 작은 버튼 (⋯ 같은)
pub fn icon_btn<'a, M: Clone + 'a>(i: Icon, active: bool) -> button::Button<'a, M> {
    let kind = if active { Kind::Flat } else { Kind::Ghost };
    button(icon(i, 15.0, p().fg2)).padding([6, 8]).style(move |_, s| button_style(kind, s))
}

// ── 입력 ───────────────────────────────────────────────────────────────

pub fn input_style(_: &Theme, status: text_input::Status) -> text_input::Style {
    let c = p();
    let border = match status {
        text_input::Status::Focused => Border { color: c.primary, width: 1.5, radius: R_BTN.into() },
        _ => Border { color: Color::TRANSPARENT, width: 1.5, radius: R_BTN.into() },
    };
    text_input::Style {
        background: Background::Color(if status == text_input::Status::Disabled { c.c1 } else { c.c2 }),
        border,
        icon: c.fg3,
        placeholder: c.fg4,
        value: c.fg,
        selection: c.primary_flat,
    }
}

/// 스타일을 입힌 입력창
pub fn input<'a, M: Clone + 'a>(placeholder: &str, value: &str) -> text_input::TextInput<'a, M> {
    text_input(placeholder, value).size(13).padding([8, 12]).style(input_style)
}

pub fn checkbox_style(_: &Theme, status: checkbox::Status) -> checkbox::Style {
    let c = p();
    let (checked, hovered) = match status {
        checkbox::Status::Active { is_checked } => (is_checked, false),
        checkbox::Status::Hovered { is_checked } => (is_checked, true),
        checkbox::Status::Disabled { is_checked } => (is_checked, false),
    };
    checkbox::Style {
        background: Background::Color(if checked { c.primary } else if hovered { c.c3 } else { c.c2 }),
        icon_color: c.on_primary,
        border: Border { radius: 5.0.into(), width: 1.0, color: if checked { c.primary } else { c.c4 } },
        text_color: Some(c.fg),
    }
}

pub fn check<'a, M: 'a>(label: impl Into<String>, on: bool) -> checkbox::Checkbox<'a, M> {
    checkbox(label, on).size(16).text_size(13).spacing(8).style(checkbox_style)
}

pub fn switch<'a, M: 'a>(on: bool) -> toggler::Toggler<'a, M> {
    toggler(on).size(20).style(|_, status| {
        let c = p();
        let on = matches!(status, toggler::Status::Active { is_toggled: true } | toggler::Status::Hovered { is_toggled: true });
        toggler::Style {
            background: if on { c.primary } else { c.c3 },
            background_border_width: 0.0,
            background_border_color: Color::TRANSPARENT,
            foreground: hex(0xFFFFFF),
            foreground_border_width: 0.0,
            foreground_border_color: Color::TRANSPARENT,
        }
    })
}

pub fn pick_style(_: &Theme, status: pick_list::Status) -> pick_list::Style {
    let c = p();
    pick_list::Style {
        text_color: c.fg,
        placeholder_color: c.fg4,
        handle_color: c.fg3,
        background: Background::Color(if status == pick_list::Status::Hovered { c.c3 } else { c.c2 }),
        border: Border { radius: R_BTN.into(), width: 0.0, color: Color::TRANSPARENT },
    }
}

pub fn progress<'a>(range_max: f32, value: f32, t: Tone) -> progress_bar::ProgressBar<'a, Theme> {
    progress_bar(0.0..=range_max.max(0.0001), value).height(4).style(move |_| progress_bar::Style {
        background: Background::Color(p().c3),
        bar: Background::Color(tone_dot_color(if t == Tone::Neutral { Tone::Primary } else { t })),
        border: radius(2.0),
    })
}

/// 세그먼트 버튼 ("다크 | 라이트 | 시스템")
pub fn segmented<'a, M: Clone + 'a, T: Clone + PartialEq + 'a>(
    options: &[(T, &'a str)],
    selected: &T,
    on_select: impl Fn(T) -> M + 'a,
) -> Element<'a, M> {
    let mut r = row![].spacing(2);
    for (value, label) in options {
        let is_sel = value == selected;
        r = r.push(
            button(text(*label).size(13).font(if is_sel { SEMIBOLD } else { REGULAR }))
                .padding([5, 12])
                .on_press(on_select(value.clone()))
                .style(move |_, s| {
                    let c = p();
                    button::Style {
                        background: Some(Background::Color(if is_sel {
                            if is_dark() { c.c3 } else { c.c1 }
                        } else if s == button::Status::Hovered {
                            c.hover
                        } else {
                            Color::TRANSPARENT
                        })),
                        text_color: if is_sel { c.fg } else { c.fg3 },
                        border: radius(R_CHIP),
                        shadow: if is_sel && !is_dark() {
                            Shadow { color: hexa(0x000000, 0.08), offset: Vector::new(0.0, 1.0), blur_radius: 2.0 }
                        } else {
                            Shadow::default()
                        },
                    }
                }),
        );
    }
    container(r)
        .padding(3)
        .style(|_| container::Style { background: Some(Background::Color(p().c2)), border: radius(R_BTN), ..Default::default() })
        .into()
}

// ── 사이드바 ───────────────────────────────────────────────────────────

pub fn nav_item<'a, M: Clone + 'a>(i: Icon, label: &'a str, active: bool, msg: M) -> Element<'a, M> {
    let c = p();
    let fg = if active { c.primary_fg } else { c.fg2 };
    button(
        row![icon(i, 16.0, fg), text(label).size(14).font(if active { SEMIBOLD } else { MEDIUM }).color(fg)]
            .spacing(10)
            .align_y(iced::Alignment::Center),
    )
    .on_press(msg)
    .width(Length::Fill)
    .padding([9, 12])
    .style(move |_, s| button::Style {
        background: Some(Background::Color(if active {
            p().primary_flat
        } else if s == button::Status::Hovered {
            p().hover
        } else {
            Color::TRANSPARENT
        })),
        text_color: fg,
        border: radius(R_ROW),
        shadow: Shadow::default(),
    })
    .into()
}

/// QR 코드 (흰 바탕 검은 칸). 휴대폰으로 찍을 수 있게 다크 테마에서도 흰 바탕을 쓴다.
pub fn qr_code<'a, M: 'a>(data: &str, module: f32) -> Element<'a, M> {
    let Ok(code) = qrcode::QrCode::new(data.as_bytes()) else {
        return Space::with_width(0).into();
    };
    let w = code.width();
    let colors = code.to_colors();
    let cell = |dark: bool| {
        container(Space::new(module, module)).width(module).height(module).style(move |_| container::Style {
            background: Some(Background::Color(if dark { hex(0x000000) } else { hex(0xFFFFFF) })),
            ..Default::default()
        })
    };
    let mut col = column![];
    for y in 0..w {
        let mut r = row![];
        for x in 0..w {
            r = r.push(cell(colors[y * w + x] == qrcode::Color::Dark));
        }
        col = col.push(r);
    }
    container(col)
        .padding(module * 3.0)
        .style(|_| container::Style { background: Some(Background::Color(hex(0xFFFFFF))), border: radius(R_ROW), ..Default::default() })
        .into()
}
