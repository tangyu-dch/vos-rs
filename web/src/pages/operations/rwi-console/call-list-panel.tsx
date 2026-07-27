import { Card, CardBody, Chip, Input, Select, SelectItem } from '@heroui/react';
import { Bot, Clock, PhoneOff, Radio, Search } from 'lucide-react';
import { motion } from 'framer-motion';
import type { CallState, LiveCallItem } from './use-rwi-websocket';
import { AudioWaveform } from './audio-waveform';

interface CallListPanelProps {
  filteredCalls: LiveCallItem[];
  currentCallId: string | undefined;
  searchQuery: string;
  filterState: string;
  onSelectCall: (callId: string) => void;
  onSearchChange: (value: string) => void;
  onFilterChange: (value: string) => void;
  renderStateChip: (state: CallState) => React.ReactNode;
  formatDuration: (sec: number) => string;
}

export function CallListPanel({
  filteredCalls,
  currentCallId,
  searchQuery,
  filterState,
  onSelectCall,
  onSearchChange,
  onFilterChange,
  renderStateChip,
  formatDuration,
}: CallListPanelProps) {
  return (
    <div className="lg:col-span-4 flex flex-col gap-3 min-h-0">
      <Card shadow="sm" className="bg-content1/80 border border-default-200/60 backdrop-blur-md flex-1 flex flex-col min-h-0">
        <CardBody className="p-4 flex flex-col gap-3 min-h-0">
          {/* Search & Filter Header */}
          <div className="flex flex-col gap-2">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <Radio className="w-4 h-4 text-primary" />
                <h3 className="text-sm font-bold text-foreground">实时会话列表</h3>
              </div>
              <Chip size="sm" variant="flat" color="primary">
                {filteredCalls.length} 个通话
              </Chip>
            </div>

            <div className="flex gap-2">
              <Input
                size="sm"
                placeholder="搜索 CallID / 主叫 / 被叫..."
                value={searchQuery}
                onValueChange={onSearchChange}
                startContent={<Search className="w-3.5 h-3.5 text-default-400" />}
                isClearable
                className="flex-1"
              />
              <Select
                size="sm"
                className="w-32"
                selectedKeys={[filterState]}
                onChange={(e) => onFilterChange(e.target.value || 'all')}
                aria-label="状态筛选"
              >
                <SelectItem key="all">全部状态</SelectItem>
                <SelectItem key="ringing">响铃中</SelectItem>
                <SelectItem key="answered">已接通</SelectItem>
                <SelectItem key="ai_active">AI 交互中</SelectItem>
                <SelectItem key="ended">已结束</SelectItem>
              </Select>
            </div>
          </div>

          {/* Call Cards List */}
          <div className="flex-1 overflow-y-auto pr-1 space-y-2.5 min-h-0">
            {filteredCalls.length === 0 ? (
              <div className="flex flex-col items-center justify-center p-8 text-center text-default-400 gap-2">
                <PhoneOff className="w-8 h-8 opacity-40" />
                <p className="text-xs font-medium">暂无匹配的实时通话</p>
              </div>
            ) : (
              filteredCalls.map((c) => {
                const isSelected = c.callId === currentCallId;
                return (
                  <motion.div
                    key={c.callId}
                    whileHover={{ scale: 1.01 }}
                    transition={{ duration: 0.15 }}
                    onClick={() => onSelectCall(c.callId)}
                    className={`cursor-pointer p-3.5 rounded-xl border transition-all duration-200 relative overflow-hidden ${
                      isSelected
                        ? 'bg-primary/10 border-primary shadow-lg shadow-primary/10'
                        : 'bg-content2/60 border-default-200/60 hover:border-default-300'
                    }`}
                  >
                    {isSelected && (
                      <div className="absolute left-0 top-0 bottom-0 w-1 bg-primary rounded-r-full" />
                    )}

                    <div className="flex items-center justify-between gap-2 mb-2">
                      <span className="font-mono text-xs font-bold text-foreground truncate">{c.callId}</span>
                      {renderStateChip(c.state)}
                    </div>

                    <div className="grid grid-cols-2 gap-2 text-xs mb-2">
                      <div>
                        <span className="text-default-400">主叫: </span>
                        <span className="font-mono font-medium text-foreground">{c.caller}</span>
                      </div>
                      <div>
                        <span className="text-default-400">被叫: </span>
                        <span className="font-mono font-medium text-foreground">{c.callee}</span>
                      </div>
                    </div>

                    {/* Agent & Duration row */}
                    <div className="flex items-center justify-between pt-2 border-t border-default-200/40 text-tiny text-default-500">
                      <div className="flex items-center gap-1 truncate max-w-[170px]">
                        <Bot className="w-3.5 h-3.5 text-primary shrink-0" />
                        <span className="truncate">{c.aiAgentName}</span>
                      </div>

                      <div className="flex items-center gap-1 font-mono text-default-400">
                        <Clock className="w-3 h-3" />
                        <span>{formatDuration(c.durationSec)}</span>
                      </div>
                    </div>

                    {/* Active waveform thumbnail if in call */}
                    {c.state !== 'ended' && (
                      <div className="mt-2 pt-2 border-t border-default-100/30 flex items-center justify-between">
                        <div className="text-[10px] text-default-400 font-mono">
                          {c.media.codec} • {c.media.packetLossPercent}% loss
                        </div>
                        <AudioWaveform
                          active={c.media.audioLevelIn > 5 || c.media.audioLevelOut > 5}
                          level={Math.max(c.media.audioLevelIn, c.media.audioLevelOut)}
                          color={c.aiStatus === 'speaking' ? 'primary' : 'success'}
                        />
                      </div>
                    )}
                  </motion.div>
                );
              })
            )}
          </div>
        </CardBody>
      </Card>
    </div>
  );
}
