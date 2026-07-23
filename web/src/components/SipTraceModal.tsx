import { useState, useEffect } from 'react';
import {
  Modal, ModalContent, ModalHeader, ModalBody, ModalFooter,
  Button, Chip, Spinner
} from '@heroui/react';
import { ArrowRight, Copy, Check, FileText, Activity } from 'lucide-react';
import { api } from '@/services/client';

export interface SipFlowEvent {
  offset_ms: number;
  message: string;
  direction: 'inbound' | 'outbound' | string;
  note: string;
  raw_message?: string;
}

interface SipTraceModalProps {
  isOpen: boolean;
  onClose: () => void;
  callId: string;
}

export function SipTraceModal({ isOpen, onClose, callId }: SipTraceModalProps) {
  const [loading, setLoading] = useState(false);
  const [events, setEvents] = useState<SipFlowEvent[]>([]);
  const [error, setError] = useState('');
  const [selectedEvent, setSelectedEvent] = useState<SipFlowEvent | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (isOpen && callId) {
      fetchTrace();
    } else {
      setEvents([]);
      setSelectedEvent(null);
      setError('');
    }
  }, [isOpen, callId]);

  const fetchTrace = async () => {
    try {
      setLoading(true);
      setError('');
      const data = await api.get<SipFlowEvent[]>(`/calls/${callId}/sip-trace`);
      setEvents(data);
      if (data.length > 0) {
        setSelectedEvent(data[0]);
      }
    } catch (e) {
      if (e instanceof Error) setError(e.message);
    } finally {
      setLoading(false);
    }
  };

  const copyRawText = (text: string) => {
    navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const getBadgeColor = (msg: string) => {
    if (msg.startsWith('200') || msg === '200 OK') return 'success';
    if (msg.startsWith('100') || msg.startsWith('180') || msg.startsWith('183')) return 'primary';
    if (msg.startsWith('4') || msg.startsWith('5') || msg.startsWith('6')) return 'danger';
    if (msg === 'INVITE') return 'warning';
    if (msg === 'BYE') return 'primary';
    return 'default';
  };

  return (
    <Modal isOpen={isOpen} onOpenChange={(o) => !o && onClose()} size="4xl" scrollBehavior="inside">
      <ModalContent className="max-w-5xl">
        <ModalHeader className="flex items-center gap-2 text-slate-800 border-b border-slate-100 pb-3">
          <Activity className="w-5 h-5 text-primary" />
          <div className="flex flex-col">
            <span className="text-base font-bold">SIP 交互信令梯形图 (SIP Flow Ladder Diagram)</span>
            <span className="text-xs font-mono font-normal text-slate-400">Call-ID: {callId}</span>
          </div>
        </ModalHeader>
        <ModalBody className="py-4">
          {loading ? (
            <div className="flex flex-col items-center justify-center py-16 gap-3">
              <Spinner size="lg" color="primary" />
              <p className="text-sm text-slate-500">正在追踪并解析全链路 SIP 报文...</p>
            </div>
          ) : error ? (
            <div className="p-4 bg-red-50 text-red-600 rounded-xl text-sm border border-red-100">
              加载失败: {error}
            </div>
          ) : events.length === 0 ? (
            <div className="text-center py-12 text-slate-400 text-sm">
              暂未捕获到该通话的 SIP 报文轨迹
            </div>
          ) : (
            <div className="grid grid-cols-1 lg:grid-cols-12 gap-6">
              {/* 梯形交互图区域 */}
              <div className="lg:col-span-7 flex flex-col gap-3">
                {/* 三节点 Header */}
                <div className="grid grid-cols-3 gap-2 text-center text-xs font-bold py-2 bg-slate-100/80 rounded-xl border border-slate-200/60">
                  <div className="text-blue-600">Caller (主叫)</div>
                  <div className="text-primary">vos-rs (Switch)</div>
                  <div className="text-emerald-600">Callee (落地/网关)</div>
                </div>

                {/* 信令交互流 */}
                <div className="flex flex-col gap-2.5 max-h-[480px] overflow-y-auto pr-1">
                  {events.map((evt, idx) => {
                    const isSelected = selectedEvent === evt;
                    const isInbound = evt.direction === 'inbound';
                    return (
                      <div
                        key={idx}
                        onClick={() => setSelectedEvent(evt)}
                        className={`p-3 rounded-xl border transition-all cursor-pointer ${
                          isSelected
                            ? 'bg-primary/10 border-primary shadow-xs ring-1 ring-primary/30'
                            : 'bg-white border-slate-200/70 hover:border-slate-300 hover:bg-slate-50/50'
                        }`}
                      >
                        <div className="flex items-center justify-between mb-1.5 text-xs text-slate-500 font-mono">
                          <span>+{evt.offset_ms} ms</span>
                          <Chip size="sm" color={getBadgeColor(evt.message)} variant="flat" className="font-bold">
                            {evt.message}
                          </Chip>
                        </div>

                        {/* 箭头示意图 */}
                        <div className="grid grid-cols-2 items-center py-1">
                          {isInbound ? (
                            <div className="flex items-center gap-1 col-span-1 border-b-2 border-primary text-primary font-mono text-xs pb-0.5">
                              <span>{evt.message}</span>
                              <ArrowRight className="w-4 h-4 ml-auto stroke-[2.5]" />
                            </div>
                          ) : (
                            <div className="flex items-center gap-1 col-start-2 border-b-2 border-emerald-400 text-emerald-600 font-mono text-xs pb-0.5">
                              <span>{evt.message}</span>
                              <ArrowRight className="w-4 h-4 ml-auto stroke-[2.5]" />
                            </div>
                          )}
                        </div>

                        <div className="text-[11px] text-slate-400 truncate mt-1">
                          {evt.note}
                        </div>
                      </div>
                    );
                  })}
                </div>
              </div>

              {/* 右侧原始 Raw SIP 报文详情 */}
              <div className="lg:col-span-5 flex flex-col bg-slate-900 text-slate-200 rounded-2xl p-4 border border-slate-800 shadow-inner">
                <div className="flex items-center justify-between border-b border-slate-800 pb-2 mb-3">
                  <span className="text-xs font-bold text-slate-300 flex items-center gap-1.5">
                    <FileText className="w-4 h-4 text-primary/70" />
                    SIP 报文详情 (Raw View)
                  </span>
                  {selectedEvent?.raw_message && (
                    <Button
                      size="sm"
                      variant="flat"
                      className="text-xs text-slate-300 bg-slate-800 hover:bg-slate-700 min-w-16 h-7"
                      onPress={() => copyRawText(selectedEvent.raw_message!)}
                    >
                      {copied ? <Check className="w-3.5 h-3.5 text-emerald-400" /> : <Copy className="w-3.5 h-3.5" />}
                      {copied ? '已复制' : '复制'}
                    </Button>
                  )}
                </div>

                {selectedEvent ? (
                  <div className="flex-1 overflow-y-auto max-h-[420px]">
                    <div className="text-xs font-mono text-primary/60 mb-2">
                      // Offset: +{selectedEvent.offset_ms}ms | Direction: {selectedEvent.direction}
                    </div>
                    <pre className="text-[11px] font-mono whitespace-pre-wrap leading-relaxed text-slate-300">
                      {selectedEvent.raw_message || `[模拟信令概览]\nMethod: ${selectedEvent.message}\nNote: ${selectedEvent.note}`}
                    </pre>
                  </div>
                ) : (
                  <div className="flex flex-col items-center justify-center flex-1 text-slate-500 text-xs py-12">
                    点击左侧信令线段查看 Raw 报文
                  </div>
                )}
              </div>
            </div>
          )}
        </ModalBody>
        <ModalFooter className="border-t border-slate-100 pt-3">
          <Button variant="flat" onPress={onClose}>
            关闭
          </Button>
        </ModalFooter>
      </ModalContent>
    </Modal>
  );
}
