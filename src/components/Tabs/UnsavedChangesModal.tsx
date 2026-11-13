import { Button, Modal } from "@heroui/react";
import { HugeiconsIcon } from "@hugeicons/react";
import { Alert01Icon } from "@hugeicons/core-free-icons";

interface UnsavedChangesModalProps {
  isOpen: boolean;
  title: string | undefined;
  onCancel: () => void;
  onDiscard: () => void;
  onSave: () => void;
}

export default function UnsavedChangesModal({ isOpen, title, onCancel, onDiscard, onSave }: UnsavedChangesModalProps) {
  return (
    <Modal>
      <Modal.Backdrop isOpen={isOpen} onOpenChange={(open) => !open && onCancel()}>
        <Modal.Container>
          <Modal.Dialog>
            <Modal.CloseTrigger />
            <Modal.Header>
              <Modal.Icon className="bg-warning-soft text-warning">
                <HugeiconsIcon icon={Alert01Icon} size={20} />
              </Modal.Icon>
              <Modal.Heading>Unsaved changes</Modal.Heading>
            </Modal.Header>
            <Modal.Body>
              <p>"{title}" has unsaved changes. Save before closing, or discard them?</p>
            </Modal.Body>
            <Modal.Footer>
              <Button variant="ghost" onPress={onCancel}>
                Cancel
              </Button>
              <Button variant="danger" onPress={onDiscard}>
                Discard
              </Button>
              <Button variant="primary" onPress={onSave}>
                Save
              </Button>
            </Modal.Footer>
          </Modal.Dialog>
        </Modal.Container>
      </Modal.Backdrop>
    </Modal>
  );
}
