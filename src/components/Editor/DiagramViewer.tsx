import { useCallback, useEffect, useRef, useState } from "react";
import { Button } from "@heroui/react";
import { HugeiconsIcon } from "@hugeicons/react";
import { Cancel01Icon, CenterFocusIcon, SearchAddIcon, SearchMinusIcon } from "@hugeicons/core-free-icons";

const MIN_SCALE = 0.1;
const MAX_SCALE = 20;
const clamp = (s: number) => Math.min(MAX_SCALE, Math.max(MIN_SCALE, s));

interface View {
  x: number;
  y: number;
  scale: number;
}

/** Full-window view of a rendered diagram: drag to pan, wheel or pinch to zoom. */
export function DiagramViewer({ svg, onClose }: { svg: SVGSVGElement; onClose: () => void }) {
  const stageRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const [view, setView] = useState<View>({ x: 0, y: 0, scale: 1 });
  const size = useRef({ w: 0, h: 0 });

  const fit = useCallback(() => {
    const stage = stageRef.current;
    if (!stage) return;
    const { w, h } = size.current;
    const scale = clamp(Math.min(stage.clientWidth / w, stage.clientHeight / h) * 0.9);
    setView({ scale, x: (stage.clientWidth - w * scale) / 2, y: (stage.clientHeight - h * scale) / 2 });
  }, []);

  /** Zooms by `factor` keeping the stage point (px, py) still. */
  const zoomAt = useCallback((factor: number, px: number, py: number) => {
    setView((v) => {
      const scale = clamp(v.scale * factor);
      const k = scale / v.scale;
      return { scale, x: px - (px - v.x) * k, y: py - (py - v.y) * k };
    });
  }, []);

  const zoomCenter = (factor: number) => {
    const stage = stageRef.current;
    if (stage) zoomAt(factor, stage.clientWidth / 2, stage.clientHeight / 2);
  };

  useEffect(() => {
    const content = contentRef.current;
    if (!content) return;
    const clone = svg.cloneNode(true) as SVGSVGElement;
    // Mermaid sizes the SVG to its container; pin it to its natural size so the
    // transform alone decides how big it is.
    const box = svg.viewBox.baseVal;
    const w = box?.width || svg.getBoundingClientRect().width;
    const h = box?.height || svg.getBoundingClientRect().height;
    size.current = { w, h };
    clone.removeAttribute("style");
    clone.setAttribute("width", String(w));
    clone.setAttribute("height", String(h));
    content.replaceChildren(clone);
    fit();
  }, [svg, fit]);

  useEffect(() => {
    const stage = stageRef.current;
    if (!stage) return;
    // Native listener: React's onWheel is passive, and pinch (ctrl+wheel) must not zoom the page.
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const rect = stage.getBoundingClientRect();
      zoomAt(Math.exp(-e.deltaY * (e.ctrlKey ? 0.01 : 0.002)), e.clientX - rect.left, e.clientY - rect.top);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    stage.addEventListener("wheel", onWheel, { passive: false });
    window.addEventListener("keydown", onKey);
    return () => {
      stage.removeEventListener("wheel", onWheel);
      window.removeEventListener("keydown", onKey);
    };
  }, [zoomAt, onClose]);

  const drag = useRef<{ x: number; y: number } | null>(null);

  return (
    <div role="dialog" aria-modal="true" aria-label="Diagram" className="fixed inset-0 z-50 flex flex-col bg-background">
      <div className="flex items-center justify-end gap-1 p-2">
        <Button size="sm" variant="ghost" isIconOnly aria-label="Zoom out" onPress={() => zoomCenter(1 / 1.25)}>
          <HugeiconsIcon icon={SearchMinusIcon} size={16} />
        </Button>
        <span className="w-12 text-center text-xs tabular-nums text-muted">{Math.round(view.scale * 100)}%</span>
        <Button size="sm" variant="ghost" isIconOnly aria-label="Zoom in" onPress={() => zoomCenter(1.25)}>
          <HugeiconsIcon icon={SearchAddIcon} size={16} />
        </Button>
        <Button size="sm" variant="ghost" isIconOnly aria-label="Fit to screen" onPress={fit}>
          <HugeiconsIcon icon={CenterFocusIcon} size={16} />
        </Button>
        <Button size="sm" variant="ghost" isIconOnly aria-label="Close" onPress={onClose}>
          <HugeiconsIcon icon={Cancel01Icon} size={16} />
        </Button>
      </div>
      <div
        ref={stageRef}
        className="milkdown relative flex-1 cursor-grab touch-none overflow-hidden active:cursor-grabbing"
        onPointerDown={(e) => {
          e.currentTarget.setPointerCapture(e.pointerId);
          drag.current = { x: e.clientX - view.x, y: e.clientY - view.y };
        }}
        onPointerMove={(e) => {
          const start = drag.current;
          if (start) setView((v) => ({ ...v, x: e.clientX - start.x, y: e.clientY - start.y }));
        }}
        onPointerUp={() => (drag.current = null)}
        onPointerCancel={() => (drag.current = null)}
        onDoubleClick={fit}
      >
        {/* `.milkdown .preview` keeps the theme's diagram restyling (milkdown-heroui-theme.css). */}
        <div
          ref={contentRef}
          className="preview absolute left-0 top-0 origin-top-left"
          style={{ transform: `translate(${view.x}px, ${view.y}px) scale(${view.scale})` }}
        />
      </div>
    </div>
  );
}
