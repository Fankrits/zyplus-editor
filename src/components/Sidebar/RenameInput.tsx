import { Input } from "@heroui/react";
import type { NodeApi } from "react-arborist";
import type { TreeNode } from "../../state/workspaceReducer";

/** The inline name field while a tree row is being renamed. Loaded the first time a rename starts. */
export default function RenameInput({ node, isFile }: { node: NodeApi<TreeNode>; isFile: boolean }) {
  return (
    <Input
      defaultValue={node.data.name}
      autoFocus
      fullWidth
      onFocus={(e) => {
        // Select just the filename stem (like Finder/VS Code) so retyping
        // doesn't clobber the extension by accident. Folders have no extension.
        const dotIndex = node.data.name.lastIndexOf(".");
        if (isFile && dotIndex > 0) {
          e.currentTarget.setSelectionRange(0, dotIndex);
        } else {
          e.currentTarget.select();
        }
      }}
      onBlur={() => node.reset()}
      onKeyDown={(e) => {
        if (e.key === "Escape") node.reset();
        if (e.key === "Enter") node.submit(e.currentTarget.value);
      }}
      className="h-7 px-2 py-0 text-sm"
    />
  );
}
