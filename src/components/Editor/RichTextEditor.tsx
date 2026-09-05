import { useEffect, useRef } from "react";
import { Crepe } from "@milkdown/crepe";
import "@milkdown/crepe/theme/common/style.css";
import "@milkdown/crepe/theme/classic.css";
import "./milkdown-heroui-theme.css";

interface RichTextEditorProps {
  initialValue: string;
  onChange: (markdown: string) => void;
}

export function RichTextEditor({ initialValue, onChange }: RichTextEditorProps) {
  const rootRef = useRef<HTMLDivElement>(null);
  const onChangeRef = useRef(onChange);
  onChangeRef.current = onChange;

  useEffect(() => {
    if (!rootRef.current) return;
    const crepe = new Crepe({
      root: rootRef.current,
      defaultValue: initialValue,
      featureConfigs: {
        [Crepe.Feature.BlockEdit]: {
          blockHandle: {
            getOffset: () => (window.innerWidth < 640 ? 4 : 12),
          },
        },
      },
    });
    crepe.on((listener) => {
      listener.markdownUpdated((_ctx, markdown, prevMarkdown) => {
        if (markdown !== prevMarkdown) onChangeRef.current(markdown);
      });
    });
    crepe.create();
    return () => {
      crepe.destroy();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return <div ref={rootRef} className="h-full overflow-y-auto px-1 sm:px-6 py-2 sm:py-6" />;
}
