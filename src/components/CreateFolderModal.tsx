import { Button, Input, Label, Modal, TextField } from "@heroui/react";
import { HugeiconsIcon } from "@hugeicons/react";
import { FolderAddIcon } from "@hugeicons/core-free-icons";

interface CreateFolderModalProps {
  isOpen: boolean;
  name: string;
  error: string | null;
  onNameChange: (name: string) => void;
  onCreate: () => void;
  onClose: () => void;
}

export default function CreateFolderModal({ isOpen, name, error, onNameChange, onCreate, onClose }: CreateFolderModalProps) {
  return (
    <Modal>
      <Modal.Backdrop isOpen={isOpen} onOpenChange={(open) => !open && onClose()}>
        <Modal.Container size="sm">
          <Modal.Dialog>
            <Modal.CloseTrigger />
            <Modal.Header>
              <Modal.Icon className="size-8 bg-accent-soft text-accent">
                <HugeiconsIcon icon={FolderAddIcon} size={16} />
              </Modal.Icon>
              <Modal.Heading>New Folder</Modal.Heading>
            </Modal.Header>
            <Modal.Body>
              <TextField>
                <Label className="mb-1.5 block">Folder name</Label>
                <Input
                  value={name}
                  onChange={(e) => onNameChange(e.target.value)}
                  placeholder="folder-name"
                  autoFocus
                  onKeyDown={(e) => {
                    if (e.key === "Enter") onCreate();
                  }}
                />
                {error && <p className="mt-1.5 text-xs text-danger">{error}</p>}
              </TextField>
            </Modal.Body>
            <Modal.Footer>
              <Button size="sm" variant="ghost" onPress={onClose}>
                Cancel
              </Button>
              <Button size="sm" variant="primary" onPress={onCreate} isDisabled={!name.trim() || error !== null}>
                Create
              </Button>
            </Modal.Footer>
          </Modal.Dialog>
        </Modal.Container>
      </Modal.Backdrop>
    </Modal>
  );
}
