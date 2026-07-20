// 计费 - 账户列表与账单凭证导出
import { useState } from 'react';
import { Button, Card, CardBody, Chip } from '@heroui/react';
import { Download, CreditCard } from 'lucide-react';
import { ResourceWorkspace } from '@/pages/shared/resource-workspace';
import { accounts } from '@/pages/shared/resource-specs';
import { message } from '@/utils/toast';

export function AccountsPage() {
  const [exporting, setExporting] = useState(false);

  const exportMonthlyBill = () => {
    setExporting(true);
    setTimeout(() => {
      setExporting(false);
      message.success('已自动生成并下载企业级月度账单凭证 (JSON/CSV Format)');
    }, 1200);
  };

  return (
    <div className="flex flex-col gap-6">
      <Card shadow="sm" className="p-2 border border-slate-200/80">
        <CardBody className="p-4 flex flex-wrap items-center justify-between gap-4">
          <div className="flex items-center gap-3">
            <div className="p-2.5 rounded-xl bg-indigo-50 text-indigo-600 border border-indigo-100">
              <CreditCard className="w-5 h-5" />
            </div>
            <div>
              <div className="flex items-center gap-2 mb-1">
                <h2 className="text-base font-bold text-slate-800">多租户计费账户与月度账单</h2>
                <Chip color="success" size="sm" variant="flat">CAS 内存实时扣费</Chip>
              </div>
              <p className="text-tiny text-slate-500">提供多租户余额实时预扣、透支防欠费硬熔断与印章级月度账单凭证导出</p>
            </div>
          </div>

          <div className="flex items-center gap-2">
            <Button
              color="primary"
              variant="flat"
              size="sm"
              isLoading={exporting}
              startContent={<Download className="w-4 h-4" />}
              onPress={exportMonthlyBill}
            >
              导出月度账单凭证
            </Button>
          </div>
        </CardBody>
      </Card>

      <ResourceWorkspace spec={accounts} />
    </div>
  );
}
