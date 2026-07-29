import {
  Button,
  Input,
  Modal,
  ModalBody,
  ModalContent,
  ModalFooter,
  ModalHeader,
} from '@heroui/react';
import { PhoneForwarded } from 'lucide-react';

interface TransferModalProps {
  isOpen: boolean;
  onClose: () => void;
  transferTarget: string;
  onTargetChange: (value: string) => void;
  onSubmit: () => void;
}

export function TransferModal({
  isOpen,
  onClose,
  transferTarget,
  onTargetChange,
  onSubmit,
}: TransferModalProps) {
  return (
    <Modal isOpen={isOpen} onClose={onClose}>
      <ModalContent>
        <ModalHeader className="flex items-center gap-2">
          <PhoneForwarded className="w-5 h-5 text-success" />
          <span>执行呼叫转移</span>
        </ModalHeader>
        <ModalBody className="gap-3">
          <p className="text-xs text-default-500">将当前通话盲转至座席分机、队列或外部中继号码。</p>
          <Input
            label="目标分机 / 号码"
            placeholder="如 8002, 1001, 或外部手机号"
            value={transferTarget}
            onValueChange={onTargetChange}
            autoFocus
          />
        </ModalBody>
        <ModalFooter>
          <Button variant="flat" onPress={onClose}>
            取消
          </Button>
          <Button color="success" onPress={onSubmit}>
            确认转接
          </Button>
        </ModalFooter>
      </ModalContent>
    </Modal>
  );
}
