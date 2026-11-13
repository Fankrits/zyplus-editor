import { Button, Modal } from "@heroui/react";
import { HugeiconsIcon } from "@hugeicons/react";
import { Alert01Icon } from "@hugeicons/core-free-icons";

interface DeleteModalProps {
  /** What is about to be deleted, or null when the dialog is closed. */
  items: readonly { data: { name: string } }[] | null;
  onCancel: () => void;
  onConfirm: () => void;
}

export default function DeleteModal({ items, onCancel, onConfirm }: DeleteModalProps) {
  return (
    <Modal>
      <Modal.Backdrop isOpen={items !== null} onOpenChange={(open) => !open && onCancel()}>
        <Modal.Container>
          <Modal.Dialog>
            <Modal.CloseTrigger />
            <Modal.Header>
              <Modal.Icon className="bg-danger-soft text-danger">
                <HugeiconsIcon icon={Alert01Icon} size={20} />
              </Modal.Icon>
              <Modal.Heading>Delete {items && items.length > 1 ? `${items.length} items` : items?.[0]?.data.name}?</Modal.Heading>
            </Modal.Header>
            <Modal.Body>
              <p>This can&apos;t be undone.</p>
            </Modal.Body>
            <Modal.Footer>
              <Button variant="ghost" onPress={onCancel}>
                Cancel
              </Button>
              <Button variant="danger" onPress={onConfirm}>
                Delete
              </Button>
            </Modal.Footer>
          </Modal.Dialog>
        </Modal.Container>
      </Modal.Backdrop>
    </Modal>
  );
}
