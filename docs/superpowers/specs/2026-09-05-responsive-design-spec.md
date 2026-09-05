# Responsive Design Specification: Mobile Drawer & Adaptive Layout

## 1. Overview & Goals
Zyplus Editor currently features a desktop-first, multi-column shell with a fixed-width (64rem/256px) sidebar, an inline tab bar, and editor panes. On narrow windows or mobile devices (<768px), this fixed layout compresses the editing workspace, overflows tab actions, and makes navigation difficult.

This specification defines a responsive layout architecture that:
1. Reuses HeroUI v3's compound `Drawer` component (`@heroui/react`) to provide an off-canvas navigation overlay on mobile/narrow viewports (<768px).
2. Maintains the existing collapsible inline sidebar on desktop/tablet viewports (≥768px) with session storage persistence.
3. Automatically closes the mobile drawer when a user opens a file or creates an item, smoothly restoring full-screen editor focus.
4. Adapts the `TabBar` by shrinking tab label limits and switching the "Rich / Plain" editor mode controls into compact icon-only buttons on mobile.
5. Adapts editor padding (Milkdown and CodeMirror) to maximize writable space on smaller viewports.

---

## 2. Architecture & Viewport Management

### Breakpoints
- **Mobile (`< 768px` / `< md`)**:
  - Inline sidebar is hidden.
  - Sidebar toggle button in `TabBar` triggers the mobile `Drawer` overlay.
  - Tab titles have reduced max width (`max-w-[8rem]`).
  - Mode switch buttons ("Rich" / "Plain") display icon-only.
  - Editor container gutters are reduced from `px-8 py-6` to `px-4 py-4`.
- **Desktop (`≥ 768px` / `≥ md`)**:
  - Persistent collapsible inline sidebar.
  - Sidebar toggle button controls desktop inline visibility (persisted in session storage).
  - Tab titles expand up to `max-w-[14rem]`.
  - Mode switch buttons display icon + text label.
  - Editor container gutters are `px-8 py-6`.

### `useMediaQuery` Hook (`src/lib/useMediaQuery.ts`)
A dedicated hook utilizing `window.matchMedia`:
- `useMediaQuery(query: string): boolean`
- `useIsDesktop(): boolean` (evaluates `(min-width: 768px)`)

Includes server-side / test environment fallback when `window.matchMedia` is not present.

---

## 3. Component Details

### 3.1 AppShell & Sidebar (`src/App.tsx`, `src/components/Sidebar/Sidebar.tsx`)
- Extract sidebar body and file tree into a reusable `SidebarContent` component.
- In `AppShell`:
  - Determine `isDesktop = useIsDesktop()`.
  - Maintain `isMobileDrawerOpen` state (defaults to `false`).
  - Pass `onToggleSidebar` to `TabBar`:
    - If `isDesktop`: toggles `isSidebarCollapsed` (saved in session storage).
    - If `!isDesktop`: toggles `isMobileDrawerOpen`.
  - Render desktop `<Sidebar>` when `isDesktop && !isSidebarCollapsed`.
  - Render mobile HeroUI `<Drawer>` when `!isDesktop`:
    - Configured with `isOpen={isMobileDrawerOpen}` and `onOpenChange={setIsMobileDrawerOpen}`.
    - `Drawer.Content placement="left"` with `Drawer.Dialog`, `Drawer.Header`, `Drawer.CloseTrigger`, and `Drawer.Body`.
    - Auto-closes on file selection or item creation.

### 3.2 TabBar (`src/components/Tabs/TabBar.tsx`)
- Accepts `isDesktop: boolean` and `isMobileDrawerOpen: boolean` (or derives via hook).
- `SidebarLeftIcon` toggle button:
  - If `isDesktop`: active styling when `!isSidebarCollapsed`.
  - If `!isDesktop`: active styling when `isMobileDrawerOpen`.
- Tab items:
  - Responsive truncation `max-w-[8rem] sm:max-w-[14rem]`.
- Rich / Plain mode buttons:
  - Uses `isIconOnly={!isDesktop}`.
  - On mobile, only renders `HugeiconsIcon` with descriptive `aria-label`.
  - On desktop, renders both icon and text label.

### 3.3 Editor Panes (`src/components/Editor/`)
- `RichTextEditor.tsx`:
  - Updates root container to `px-4 sm:px-8 py-4 sm:py-6`.
- `plainTextTheme.ts`:
  - Updates `.cm-content` padding using responsive media query:
    - Base (<640px): `padding: "1.5rem 1rem 8rem 1rem"`
    - `@media (min-width: 640px)`: `padding: "3rem 1.5rem 12rem 1.5rem"`

---

## 4. Testing & Verification
- Unit test for `useMediaQuery` / `useIsDesktop` with mocked `matchMedia`.
- Build verification with `bun run build` to ensure clean TypeScript compilation.
- Functional automated tests with `bun test`.
- Web / browser verification at mobile viewport (375x667, 390x844) and desktop viewport (1280x800) using `agent-browser`.
