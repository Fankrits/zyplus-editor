import { useEffect } from "react";
import "./App.css";
import { WorkspaceProvider, useWorkspace } from "./state/workspaceStore";
import { writeTextFile } from "./lib/fs";
import { Sidebar } from "./components/Sidebar/Sidebar";
import { TabBar } from "./components/Tabs/TabBar";
import { EditorPane } from "./components/Editor/EditorPane";

function AppShell() {
  const { activeTab, dispatch } = useWorkspace();

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "s") {
        e.preventDefault();
        if (!activeTab || !activeTab.isDirty) return;
        writeTextFile(activeTab.filePath, activeTab.content).then(() => {
          dispatch({ type: "SAVE_TAB_SUCCESS", id: activeTab.id });
        });
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [activeTab, dispatch]);

  return (
    <div className="flex h-screen w-screen overflow-hidden bg-white text-black dark:bg-neutral-900 dark:text-white">
      <Sidebar />
      <div className="flex min-w-0 flex-1 flex-col">
        <TabBar />
        <EditorPane />
      </div>
    </div>
  );
}

function App() {
  return (
    <WorkspaceProvider>
      <AppShell />
    </WorkspaceProvider>
  );
}

export default App;
