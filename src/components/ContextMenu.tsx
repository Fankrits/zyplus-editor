import { useCallback, useEffect, useRef, useState, type TouchEvent as ReactTouchEvent, lazy } from "react";
import { OnDemand } from "./OnDemand";
import type { IconSvgElement } from "@hugeicons/react";

export interface ContextMenuItem {
  key: string;
  label: string;
  icon?: IconSvgElement;
  danger?: boolean;
  disabled?: boolean;
  onSelect: () => void;
}

export interface ContextMenuState {
  x: number;
  y: number;
  items: ContextMenuItem[];
}

/**
 * Where a menu was summoned from. A React mouse event satisfies this as-is;
 * `useLongPress` synthesizes one so touch can open the very same menu.
 */
export interface ContextMenuOrigin {
  clientX: number;
  clientY: number;
  preventDefault: () => void;
  stopPropagation: () => void;
}

/** Tracks the {x, y, items} for a context menu. One instance per menu "owner" (file tree, tab bar, ...). */
export function useContextMenu() {
  const [state, setState] = useState<ContextMenuState | null>(null);

  const open = useCallback((e: ContextMenuOrigin, items: ContextMenuItem[]) => {
    e.preventDefault();
    e.stopPropagation();
    setState({ x: e.clientX, y: e.clientY, items });
  }, []);

  const close = useCallback(() => setState(null), []);

  return { state, open, close };
}

/** Movement past this many pixels means the finger is scrolling, not pressing. */
const LONG_PRESS_SLOP = 10;

/**
 * Touch screens have no right-click, which left rename / duplicate / delete
 * unreachable on them. Holding a row still opens the same menu, at the point
 * held; sliding out of it cancels so the list stays scrollable.
 */
export function useLongPress(onLongPress: (origin: ContextMenuOrigin) => void, delayMs = 500) {
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const origin = useRef<{ x: number; y: number } | null>(null);
  const handler = useRef(onLongPress);
  handler.current = onLongPress;

  const cancel = useCallback(() => {
    if (timer.current !== null) {
      clearTimeout(timer.current);
      timer.current = null;
    }
    origin.current = null;
  }, []);

  // A press still pending when the row unmounts must not fire into a dead tree.
  useEffect(() => cancel, [cancel]);

  const onTouchStart = useCallback(
    (e: ReactTouchEvent) => {
      const touch = e.touches[0];
      if (!touch || e.touches.length > 1) return cancel();
      const { clientX, clientY } = touch;
      origin.current = { x: clientX, y: clientY };
      timer.current = setTimeout(() => {
        timer.current = null;
        handler.current({
          clientX,
          clientY,
          preventDefault: () => {},
          stopPropagation: () => {},
        });
      }, delayMs);
    },
    [cancel, delayMs],
  );

  const onTouchMove = useCallback(
    (e: ReactTouchEvent) => {
      const touch = e.touches[0];
      const from = origin.current;
      if (!touch || !from) return;
      if (
        Math.abs(touch.clientX - from.x) > LONG_PRESS_SLOP ||
        Math.abs(touch.clientY - from.y) > LONG_PRESS_SLOP
      ) {
        cancel();
      }
    },
    [cancel],
  );

  return { onTouchStart, onTouchMove, onTouchEnd: cancel, onTouchCancel: cancel };
}

const ContextMenuPopover = lazy(() => import("./ContextMenuPopover"));

/**
 * A right-click menu anchored at `state.x/y`. The menu itself (HeroUI's Dropdown and react-aria's
 * popover and menu code) loads the first time one is opened, not at startup.
 */
export function ContextMenu({ state, onClose }: { state: ContextMenuState | null; onClose: () => void }) {
  return (
    <OnDemand when={state !== null}>
      <ContextMenuPopover state={state} onClose={onClose} />
    </OnDemand>
  );
}
