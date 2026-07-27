import { Button, Chip, Input, Modal, ModalBody, ModalContent, ModalFooter, ModalHeader } from '@heroui/react';
import { Mic, Send } from 'lucide-react';

interface SpeakModalProps {
  isOpen: boolean;
  onClose: () => void;
  speakText: string;
  onTextChange: (value: string) => void;
  onSubmit: () => void;
}

const QUICK_REPLIES = [
  '请您稍等，我立即为您转接人工客服',
  '您的身份验证已通过，请问还有其他需求吗？',
  '十分抱歉给您带来不便，我们将优先处理。',
];

export function SpeakModal({ isOpen, onClose, speakText, onTextChange, onSubmit }: SpeakModalProps) {
  return (
    <Modal isOpen={isOpen} onClose={onClose}>
      <ModalContent>
        <ModalHeader className="flex items-center gap-2">
          <Mic className="w-5 h-5 text-primary" />
          <span>AI Voice Agent 注入 TTS 合成播报</span>
        </ModalHeader>
        <ModalBody className="gap-3">
          <p className="text-xs text-default-500">
            手动输入的文本将通过实时 TTS 引擎合成音频并直接注入当前通话媒体通道。
          </p>
          <Input
            label="TTS 播报文本"
            placeholder="请输入需要 AI 语音播报的内容..."
            value={speakText}
            onValueChange={onTextChange}
            autoFocus
          />
          <div className="flex flex-wrap gap-1.5 pt-2">
            <span className="text-tiny text-default-400 w-full mb-1">快捷回复预设:</span>
            {QUICK_REPLIES.map((phrase) => (
              <Chip
                key={phrase}
                size="sm"
                variant="flat"
                className="cursor-pointer hover:bg-primary/20"
                onClick={() => onTextChange(phrase)}
              >
                {phrase}
              </Chip>
            ))}
          </div>
        </ModalBody>
        <ModalFooter>
          <Button variant="flat" onPress={onClose}>
            取消
          </Button>
          <Button color="secondary" onPress={onSubmit} startContent={<Send className="w-4 h-4" />}>
            确认发送播报
          </Button>
        </ModalFooter>
      </ModalContent>
    </Modal>
  );
}
