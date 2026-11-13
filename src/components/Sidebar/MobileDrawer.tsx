import { Drawer } from "@heroui/react";
import { SidebarContent } from "./Sidebar";

type SidebarContentProps = Parameters<typeof SidebarContent>[0];

interface MobileDrawerProps {
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
  onRequestCreate: SidebarContentProps["onRequestCreate"];
  onOpenFile: SidebarContentProps["onOpenFile"];
  onOpenFolder: SidebarContentProps["onOpenFolder"];
}

export default function MobileDrawer({ isOpen, onOpenChange, onRequestCreate, onOpenFile, onOpenFolder }: MobileDrawerProps) {
  return (
    <Drawer isOpen={isOpen} onOpenChange={onOpenChange}>
      <Drawer.Backdrop>
        <Drawer.Content placement="left" className="w-72 max-w-[85vw] h-full p-0">
          <Drawer.Dialog className="h-full flex flex-col bg-background text-foreground">
            <Drawer.Header className="flex items-center justify-between border-b border-border px-3 py-2">
              <Drawer.Heading className="text-sm font-semibold truncate">Files</Drawer.Heading>
              <Drawer.CloseTrigger />
            </Drawer.Header>
            <Drawer.Body className="flex-1 p-0 overflow-hidden">
              <SidebarContent onRequestCreate={onRequestCreate} onOpenFile={onOpenFile} onOpenFolder={onOpenFolder} />
            </Drawer.Body>
          </Drawer.Dialog>
        </Drawer.Content>
      </Drawer.Backdrop>
    </Drawer>
  );
}
