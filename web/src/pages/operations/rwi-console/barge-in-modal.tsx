import { Button, Modal, ModalBody, ModalContent, ModalFooter, ModalHeader } from '@heroui/react';
import { AlertTriangle } from 'lucide-react';

interface BargeInModalProps {
  isOpen: boolean;
  onClose: () => void;
  onConfirm: () => void;
}

export function BargeInModal({ isOpen, onClose, onConfirm }: BargeInModalProps) {
  return (
    <Modal isOpen={isOpen} onClose={onClose}>
      <ModalContent>
        <ModalHeader className="flex items-center gap-2 text-danger">
          <AlertTriangle className="w-5 h-5" />
          <span>确认执行 BargeIn 强插抢断？</span>
        </ModalHeader>
        <ModalBody>
          <p className="text-xs text-default-400">
            该指令会立即中断 AI Voice Agent 的当前合成输出，强制将音频流切换为坐席直连模式。
          </p>
        </ModalBody>
        <ModalFooter>
          <Button variant="flat" onPress={onClose}>
            取消
          </Button>
          <Button color="danger" onPress={onConfirm}>
            确认强插 (BargeIn)
          </Button>
        </ModalFooter>
      </ModalContent>
    </Modal>
  );
}
