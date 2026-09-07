import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type TouchEvent as ReactTouchEvent,
} from "react";
import { Dropdown } from "@heroui/react";
import { HugeiconsIcon } from "@hugeicons/react";
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

/**
 * Renders a right-click menu anchored at `state.x/y`. The anchor is a real
 * (but invisible, 1x1) HeroUI Dropdown.Trigger positioned at the click point —
 * react-aria-components' Popover positions relative to its trigger's DOM rect,
 * so this is what makes the menu appear at the cursor instead of a fixed button.
 */
export function ContextMenu({ state, onClose }: { state: ContextMenuState | null; onClose: () => void }) {
  return (
    <Dropdown isOpen={state !== null} onOpenChange={(open) => !open && onClose()}>
      <Dropdown.Trigger
        aria-hidden="true"
        excludeFromTabOrder
        style={{ position: "fixed", left: state?.x ?? 0, top: state?.y ?? 0, width: 1, height: 1 }}
        className="pointer-events-none opacity-0"
      />
      <Dropdown.Popover placement="bottom start">
        <Dropdown.Menu>
          {(state?.items ?? []).map((item) => (
            <Dropdown.Item
              key={item.key}
              id={item.key}
              textValue={item.label}
              isDisabled={item.disabled}
              variant={item.danger ? "danger" : "default"}
              onAction={item.onSelect}
            >
              <span className="flex items-center gap-2">
                {item.icon && <HugeiconsIcon icon={item.icon} size={16} strokeWidth={1.75} />}
                {item.label}
              </span>
            </Dropdown.Item>
          ))}
        </Dropdown.Menu>
      </Dropdown.Popover>
    </Dropdown>
  );
}
