import { Button, Modal, ModalBody, ModalContent, ModalFooter, ModalHeader } from '@heroui/react';

export function ConfirmDialog({
  open,
  title,
  message,
  loading,
  onConfirm,
  onClose,
}: {
  open: boolean;
  title: string;
  message: string;
  loading?: boolean;
  onConfirm: () => void;
  onClose: () => void;
}) {
  return (
    <Modal isOpen={open} onOpenChange={(o) => !o && onClose()} size="sm">
      <ModalContent>
        <ModalHeader>{title}</ModalHeader>
        <ModalBody>
          <p className="text-small text-default-500">{message}</p>
        </ModalBody>
        <ModalFooter>
          <Button variant="flat" onPress={onClose}>
            取消
          </Button>
          <Button color="danger" isLoading={loading} onPress={onConfirm}>
            确认
          </Button>
        </ModalFooter>
      </ModalContent>
    </Modal>
  );
}
