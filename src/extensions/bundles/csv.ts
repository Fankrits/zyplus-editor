// @ts-expect-error - No types available for tabulator-tables in this setup
import { TabulatorFull as Tabulator } from "tabulator-tables";
import Papa from "papaparse";
import type { ExtensionRuntime } from "../types";
import { EditorView, basicSetup } from "codemirror";
import { EditorState } from "@codemirror/state";

function serialize(rows: string[][], meta: Papa.ParseMeta, hasBOM: boolean, trailingNewline: string) {
  const unparsed = Papa.unparse(rows, {
    quotes: false,
    delimiter: meta.delimiter || ",",
    newline: meta.linebreak || "\n",
  });
  const bom = hasBOM ? "\uFEFF" : "";
  // Papa.unparse naturally doesn't add a trailing newline if rows are empty or normally.
  // Wait, Papa.unparse joins rows with newline. The last row doesn't have a newline.
  return bom + unparsed + trailingNewline;
}

export default function createCsvExtension(): ExtensionRuntime {
  return {
    id: "csv",
    
    renderCodeBlockPreview(_language, content, _applyPreview) {
      // code-fence preview as lightweight HTML table
      const parsed = Papa.parse<string[]>(content.trim(), { skipEmptyLines: false });
      const rows = parsed.data;
      if (rows.length === 0) return null;
      
      const container = document.createElement("div");
      container.className = "csv-preview-container";
      container.style.overflowX = "auto";
      
      const table = document.createElement("table");
      table.className = "csv-preview-table";
      table.style.borderCollapse = "collapse";
      table.style.width = "100%";
      
      rows.forEach((row, rowIndex) => {
        const tr = document.createElement("tr");
        row.forEach(cell => {
          const el = document.createElement(rowIndex === 0 ? "th" : "td");
          el.textContent = cell;
          el.style.border = "1px solid var(--separator)";
          el.style.padding = "4px 8px";
          el.style.textAlign = "left";
          if (rowIndex === 0) {
            el.style.background = "var(--surface-secondary)";
          }
          tr.appendChild(el);
        });
        table.appendChild(tr);
      });
      
      container.appendChild(table);
      return container;
    },

    mountFileEditor(host, { initialValue, onChange, mode, onModeChange }) {
      let hasBOM = initialValue.startsWith("\uFEFF");
      // Papa Parse strips BOM, but we also can manually slice it just to be safe if we want to sniff trailing newline
      const withoutBOM = hasBOM ? initialValue.slice(1) : initialValue;
      
      let trailingNewline = "";
      if (withoutBOM.endsWith("\r\n")) trailingNewline = "\r\n";
      else if (withoutBOM.endsWith("\n")) trailingNewline = "\n";
      else if (withoutBOM.endsWith("\r")) trailingNewline = "\r";
      
      const trimmed = trailingNewline ? withoutBOM.slice(0, -trailingNewline.length) : withoutBOM;

      let parsed = Papa.parse<string[]>(trimmed, { skipEmptyLines: false });
      let rows = parsed.data;
      let meta = parsed.meta;
      
      let current = mode || (parsed.errors.length > 0 ? "text" : "table");
      onModeChange(current);

      let table: Tabulator | null = null;
      let cm: EditorView | null = null;
      
      const updateData = () => {
        onChange(serialize(rows, parsed.meta, hasBOM, trailingNewline));
      };

      const renderTable = () => {
        host.innerHTML = "";
        
        let maxCols = 0;
        for (const r of rows) {
          if (r.length > maxCols) maxCols = r.length;
        }
        
        const headers = rows.length > 0 ? rows[0] : [];
        const columns = [];
        for (let i = 0; i < maxCols; i++) {
          columns.push({
            title: i < headers.length ? headers[i] : `Column ${i + 1}`,
            field: String(i),
            editor: "input",
            editableTitle: true,
            headerSort: false,
          });
        }
        
        const tableData = rows.slice(1).map((row, idx) => {
          const obj: any = { _id: idx + 1 };
          for (let i = 0; i < maxCols; i++) {
            obj[i] = row[i] !== undefined ? row[i] : "";
          }
          return obj;
        });

        table = new Tabulator(host, {
          data: tableData,
          columns,
          layout: "fitData",
          reactiveData: false,
          history: true,
        });

        table.on("cellEdited", (cell: any) => {
          const rowId = cell.getRow().getData()._id;
          const colField = cell.getColumn().getField();
          const val = cell.getValue();
          
          if (!rows[rowId]) {
             // In case a row was added
             rows[rowId] = [];
          }
          rows[rowId][parseInt(colField)] = val;
          updateData();
        });

        table.on("columnTitleChanged", (column: any) => {
          const colField = column.getField();
          const val = column.getDefinition().title;
          if (rows.length === 0) {
            rows.push([]);
          }
          rows[0][parseInt(colField)] = val;
          updateData();
        });
      };

      const renderText = () => {
        host.innerHTML = "";
        const state = EditorState.create({
          doc: serialize(rows, parsed.meta, hasBOM, trailingNewline),
          extensions: [
            basicSetup,
            EditorView.updateListener.of((update) => {
              if (update.docChanged) {
                onChange(update.state.doc.toString());
              }
            })
          ],
        });
        
        cm = new EditorView({
          state,
          parent: host,
        });
      };

      if (current === "table") {
        renderTable();
      } else {
        renderText();
      }

      return {
        _test_getOutput: () => serialize(rows, meta, hasBOM, trailingNewline),
        _test_getCm: () => cm,
        _test_getTable: () => table,
        destroy() {
          if (table) table.destroy();
          if (cm) cm.destroy();
          host.innerHTML = "";
        },
        setMode(next) {
          if (next === current) return;
          
          if (current === "text" && cm) {
            const currentText = cm.state.doc.toString();
            hasBOM = currentText.startsWith("\uFEFF");
            const wBOM = hasBOM ? currentText.slice(1) : currentText;
            trailingNewline = "";
            if (wBOM.endsWith("\r\n")) trailingNewline = "\r\n";
            else if (wBOM.endsWith("\n")) trailingNewline = "\n";
            else if (wBOM.endsWith("\r")) trailingNewline = "\r";
            const t = trailingNewline ? wBOM.slice(0, -trailingNewline.length) : wBOM;
            parsed = Papa.parse<string[]>(t, { skipEmptyLines: false });
            rows = parsed.data;
            meta = parsed.meta;
          }
          
          current = next;
          onModeChange(current);
          if (table) {
            table.destroy();
            table = null;
          }
          if (cm) {
            cm.destroy();
            cm = null;
          }
          if (current === "table") {
            renderTable();
          } else {
            renderText();
          }
        },
      };
    },
  };
}
