import { useCallback, useState, type MouseEvent as ReactMouseEvent } from "react";
import { Dropdown } from "@heroui/react";

export interface ContextMenuItem {
  key: string;
  label: string;
  danger?: boolean;
  disabled?: boolean;
  onSelect: () => void;
}

export interface ContextMenuState {
  x: number;
  y: number;
  items: ContextMenuItem[];
}

/** Tracks the {x, y, items} for a right-click menu. One instance per menu "owner" (file tree, tab bar, ...). */
export function useContextMenu() {
  const [state, setState] = useState<ContextMenuState | null>(null);

  const open = useCallback((e: ReactMouseEvent, items: ContextMenuItem[]) => {
    e.preventDefault();
    e.stopPropagation();
    setState({ x: e.clientX, y: e.clientY, items });
  }, []);

  const close = useCallback(() => setState(null), []);

  return { state, open, close };
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
              isDisabled={item.disabled}
              variant={item.danger ? "danger" : "default"}
              onAction={item.onSelect}
            >
              {item.label}
            </Dropdown.Item>
          ))}
        </Dropdown.Menu>
      </Dropdown.Popover>
    </Dropdown>
  );
}
