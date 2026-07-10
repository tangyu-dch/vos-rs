import { graphic, init, use } from 'echarts/core';
import { GaugeChart, LineChart, PieChart } from 'echarts/charts';
import {
  GraphicComponent,
  GridComponent,
  LegendComponent,
  TooltipComponent,
} from 'echarts/components';
import { CanvasRenderer } from 'echarts/renderers';
import type { ECharts } from 'echarts/core';

use([
  CanvasRenderer,
  GaugeChart,
  GraphicComponent,
  GridComponent,
  LegendComponent,
  LineChart,
  PieChart,
  TooltipComponent,
]);

export { graphic, init };
export type { ECharts };
