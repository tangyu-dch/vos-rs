// 号码管理 - 号码池组列表 (极简直观重构版)
import { Card, CardBody, Chip } from '@heroui/react';
import { Layers, ArrowRight, PhoneCall, Cpu } from 'lucide-react';
import { ResourceWorkspace } from '@/components/resource-workspace';
import { callerPools } from '@/pages/shared/resource-specs';

export const CallerPoolsPage = () => {
  return (
    <div className="space-y-4">
      {/* 传显式 CRS 拓扑全景指引 Banner */}
      <Card className="bg-gradient-to-r from-blue-950/40 via-purple-950/30 to-slate-900/40 border border-blue-500/20 shadow-md">
        <CardBody className="p-4">
          <div className="flex flex-col md:flex-row items-start md:items-center justify-between gap-4">
            <div className="flex items-center gap-3">
              <div className="p-2.5 rounded-xl bg-blue-500/10 border border-blue-500/20 text-blue-400">
                <Layers className="w-6 h-6" />
              </div>
              <div>
                <h3 className="text-base font-semibold text-slate-100 flex items-center gap-2">
                  CRS 号码池控制枢纽
                  <Chip size="sm" color="primary" variant="flat">极简直连模式</Chip>
                </h3>
                <p className="text-xs text-slate-400 mt-0.5">
                  两端直连，无需繁琐授权：本租户下的分机与中继可直接选用所属号码池进行轮询显号外呼。
                </p>
              </div>
            </div>

            {/* 可视化选路拓扑链 */}
            <div className="flex items-center gap-2 text-xs bg-slate-900/80 px-3.5 py-2 rounded-lg border border-slate-800">
              <div className="flex items-center gap-1.5 text-slate-300">
                <PhoneCall className="w-3.5 h-3.5 text-cyan-400" />
                <span>分机/中继</span>
              </div>
              <ArrowRight className="w-3.5 h-3.5 text-slate-500" />
              <div className="flex items-center gap-1.5 text-purple-300 font-medium">
                <Layers className="w-3.5 h-3.5 text-purple-400" />
                <span>租户号码池</span>
              </div>
              <ArrowRight className="w-3.5 h-3.5 text-slate-500" />
              <div className="flex items-center gap-1.5 text-emerald-300">
                <Cpu className="w-3.5 h-3.5 text-emerald-400" />
                <span>落地物理中继</span>
              </div>
            </div>
          </div>
        </CardBody>
      </Card>

      {/* 标准资源工作区 */}
      <ResourceWorkspace spec={callerPools} />
    </div>
  );
};
