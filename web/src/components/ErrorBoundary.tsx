import React, { Component, type ReactNode } from 'react';
import { Card, CardBody, Button } from '@heroui/react';
import { AlertTriangle, RotateCw } from 'lucide-react';

interface Props {
  children: ReactNode;
}

interface State {
  hasError: boolean;
  error?: Error;
}

export default class ErrorBoundary extends Component<Props, State> {
  constructor(props: Props) {
    super(props);
    this.state = { hasError: false };
  }

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, errorInfo: React.ErrorInfo) {
    console.error('ErrorBoundary caught an error:', error, errorInfo);
  }

  render() {
    if (this.state.hasError) {
      return (
        <div className="flex items-center justify-center min-h-screen p-6">
          <Card className="max-w-lg w-full">
            <CardBody className="gap-4">
              <div className="flex items-center gap-3 text-danger">
                <AlertTriangle className="w-6 h-6" />
                <h2 className="text-lg font-semibold">应用发生意外错误</h2>
              </div>
              <p className="text-sm text-default-500">
                {this.state.error?.message || '未知错误'}
              </p>
              <Button
                color="primary"
                variant="flat"
                startContent={<RotateCw className="w-4 h-4" />}
                onPress={() => window.location.reload()}
                className="w-fit"
              >
                重新加载页面
              </Button>
            </CardBody>
          </Card>
        </div>
      );
    }
    return this.props.children;
  }
}
