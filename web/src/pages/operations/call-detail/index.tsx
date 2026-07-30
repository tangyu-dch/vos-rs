// 运营监控 - 通话详情

import { Button } from '@heroui/react';
import { ArrowLeft, PhoneCall } from 'lucide-react';
import { useNavigate, useParams } from 'react-router-dom';
import { CallDetailView } from '@/pages/billing/call-detail';

export function CallDetailPage() {
  const navigate = useNavigate();
  const { id = '' } = useParams();

  return (
    <div className="flex flex-col gap-4">
      <div className="overview-card p-4 flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-3">
          <Button
            isIconOnly
            size="sm"
            variant="flat"
            aria-label="返回通话记录"
            onPress={() => navigate('/calls')}
          >
            <ArrowLeft className="w-4 h-4" />
          </Button>
          <div className="w-9 h-9 rounded-xl bg-primary/10 text-primary flex items-center justify-center">
            <PhoneCall className="w-4 h-4" />
          </div>
          <div>
            <h1 className="text-base font-semibold text-foreground">通话详情</h1>
            <p className="text-tiny text-default-500 mt-0.5">查看呼叫结果、媒体质量与录音按键。</p>
          </div>
        </div>
      </div>
      <CallDetailView id={id} />
    </div>
  );
}
