// 号码管理 - 号码库存 (极简简化版)
import { Card, CardBody, Chip } from '@heroui/react';
import { Phone, CheckCircle2, Building2 } from 'lucide-react';
import { ResourceWorkspace } from '@/components/resource-workspace';
import { numbers } from '@/pages/shared/resource-specs';

export const NumbersPage = () => {
  return (
    <div className="space-y-4">
      {/* 极简说明 Banner */}
      <Card className="bg-slate-900/60 border border-slate-800 shadow-sm">
        <CardBody className="p-3.5 flex flex-col md:flex-row items-start md:items-center justify-between gap-3 text-xs">
          <div className="flex items-center gap-3">
            <div className="p-2 rounded-lg bg-emerald-500/10 border border-emerald-500/20 text-emerald-400">
              <Phone className="w-5 h-5" />
            </div>
            <div>
              <span className="font-semibold text-slate-200 text-sm block">真实 DID 号码库</span>
              <span className="text-slate-400">
                填写真实号码 + 所属租户 + 物理落地中继即可保存。归属租户后的号码将自动提供给该租户的分机直接调用！
              </span>
            </div>
          </div>
          <div className="flex items-center gap-2">
            <Chip size="sm" color="success" variant="flat" startContent={<CheckCircle2 className="w-3 h-3" />}>
              免复杂授权关系
            </Chip>
            <Chip size="sm" color="secondary" variant="flat" startContent={<Building2 className="w-3 h-3" />}>
              租户资源隔离
            </Chip>
          </div>
        </CardBody>
      </Card>

      <ResourceWorkspace spec={numbers} />
    </div>
  );
};
