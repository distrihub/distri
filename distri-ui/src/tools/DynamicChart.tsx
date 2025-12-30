import React, { useState, useMemo } from 'react';
import {
  ResponsiveContainer,
  LineChart,
  BarChart,
  AreaChart,
  PieChart,
  ScatterChart,
  Line,
  Bar,
  Area,
  Pie,
  Cell,
  Scatter,
  XAxis,
  YAxis,
  CartesianGrid,
  Tooltip,
  Legend,
} from 'recharts';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { Badge } from '@/components/ui/badge';
import { DistriUiTool, UiToolProps } from '@distri/react';
import { BarChart3, LineChart as LineChartIcon, PieChart as PieChartIcon, TrendingUp } from 'lucide-react';
import { ToolResult } from '@distri/core';

// Supported chart types
type ChartType = 'line' | 'bar' | 'area' | 'pie' | 'scatter';

// Data format interfaces
interface ChartDataPoint {
  [key: string]: string | number;
}

interface ChartConfig {
  type: ChartType;
  title?: string;
  description?: string;
  data: ChartDataPoint[];
  xKey?: string;
  yKeys?: string[];
  colors?: string[];
  width?: number;
  height?: number;
  showGrid?: boolean;
  showLegend?: boolean;
  showTooltip?: boolean;
}

// Default colors for chart series
const DEFAULT_COLORS = [
  '#8884d8', '#82ca9d', '#ffc658', '#ff7300',
  '#8dd1e1', '#d084d0', '#ffb347', '#87ceeb'
];

export const DynamicChart: React.FC<UiToolProps> = ({
  toolCall,
  toolCallState,
  completeTool
}) => {
  const [currentChartType, setCurrentChartType] = useState<ChartType | null>(null);

  // Parse tool input
  const input = useMemo(() => {
    try {
      return typeof toolCall.input === 'string' ? JSON.parse(toolCall.input) : toolCall.input;
    } catch {
      return {};
    }
  }, [toolCall.input]);

  // Extract chart configuration
  const chartConfig: ChartConfig = useMemo(() => {
    const config: ChartConfig = {
      type: input.type || 'line',
      title: input.title || 'Dynamic Chart',
      description: input.description,
      data: input.data || [],
      xKey: input.xKey || input.x_key,
      yKeys: input.yKeys || input.y_keys || [],
      colors: input.colors || DEFAULT_COLORS,
      width: input.width,
      height: input.height || 400,
      showGrid: input.showGrid !== false,
      showLegend: input.showLegend !== false,
      showTooltip: input.showTooltip !== false
    };

    // Auto-detect keys if not provided
    if (config.data.length > 0) {
      const firstDataPoint = config.data[0];
      const keys = Object.keys(firstDataPoint);

      if (!config.xKey && keys.length > 0) {
        // Use first key as x-axis or look for common time/category keys
        const timeKeys = ['time', 'date', 'timestamp', 'x', 'label', 'name', 'category'];
        config.xKey = timeKeys.find(key => keys.includes(key)) || keys[0];
      }

      if (config.yKeys?.length === 0) {
        // Use all numeric keys except x-key as y-keys
        config.yKeys = keys.filter(key =>
          key !== config.xKey &&
          typeof firstDataPoint[key] === 'number'
        );
      }
    }

    return config;
  }, [input]);

  const displayType = currentChartType || chartConfig.type;

  // Handle completion
  const handleComplete = () => {
    const result: ToolResult = {
      tool_call_id: toolCall.tool_call_id,
      tool_name: toolCall.tool_name,
      parts: [{
        part_type: 'data',
        data: {
          result: `Chart rendered successfully with ${chartConfig.data.length} data points`,
          success: true,
          error: undefined
        }
      }],
    };
    completeTool(result);
  };

  // Render chart based on type
  const renderChart = () => {
    if (!chartConfig.data || chartConfig.data.length === 0) {
      return (
        <div className="flex items-center justify-center h-64 text-muted-foreground">
          <div className="text-center">
            <BarChart3 className="h-12 w-12 mx-auto mb-2 opacity-50" />
            <p>No data available to display</p>
          </div>
        </div>
      );
    }

    const commonProps = {
      data: chartConfig.data,
      margin: { top: 5, right: 30, left: 20, bottom: 5 }
    };

    switch (displayType) {
      case 'line':
        return (
          <LineChart {...commonProps}>
            {chartConfig.showGrid && <CartesianGrid strokeDasharray="3 3" />}
            <XAxis dataKey={chartConfig.xKey} />
            <YAxis />
            {chartConfig.showTooltip && <Tooltip />}
            {chartConfig.showLegend && <Legend />}
            {chartConfig.yKeys?.map((key, index) => (
              <Line
                key={key}
                type="monotone"
                dataKey={key}
                stroke={chartConfig.colors?.[index % chartConfig.colors.length]}
                strokeWidth={2}
                dot={{ r: 4 }}
              />
            ))}
          </LineChart>
        );

      case 'bar':
        return (
          <BarChart {...commonProps}>
            {chartConfig.showGrid && <CartesianGrid strokeDasharray="3 3" />}
            <XAxis dataKey={chartConfig.xKey} />
            <YAxis />
            {chartConfig.showTooltip && <Tooltip />}
            {chartConfig.showLegend && <Legend />}
            {chartConfig.yKeys?.map((key, index) => (
              <Bar
                key={key}
                dataKey={key}
                fill={chartConfig.colors?.[index % chartConfig.colors.length]}
              />
            ))}
          </BarChart>
        );

      case 'area':
        return (
          <AreaChart {...commonProps}>
            {chartConfig.showGrid && <CartesianGrid strokeDasharray="3 3" />}
            <XAxis dataKey={chartConfig.xKey} />
            <YAxis />
            {chartConfig.showTooltip && <Tooltip />}
            {chartConfig.showLegend && <Legend />}
            {chartConfig.yKeys?.map((key, index) => (
              <Area
                key={key}
                type="monotone"
                dataKey={key}
                stackId="1"
                stroke={chartConfig.colors?.[index % chartConfig.colors.length]}
                fill={chartConfig.colors?.[index % chartConfig.colors.length]}
                fillOpacity={0.6}
              />
            ))}
          </AreaChart>
        );

      case 'pie': {
        // For pie charts, use the first y-key as the value
        const valueKey = chartConfig.yKeys?.[0];
        return (
          <PieChart>
            <Pie
              data={chartConfig.data}
              cx="50%"
              cy="50%"
              labelLine={false}
              label={({ [chartConfig.xKey as string]: name, [valueKey as string]: value }: { [key: string]: string | number }) => `${name}: ${value}`}
              outerRadius={120}
              fill="#8884d8"
              dataKey={valueKey}
            >
              {chartConfig.data.map((_entry, index) => (
                <Cell
                  key={`cell-${index}`}
                  fill={chartConfig.colors?.[index % chartConfig.colors.length]}
                />
              ))}
            </Pie>
            {chartConfig.showTooltip && <Tooltip />}
          </PieChart>
        );
      }

      case 'scatter':
        return (
          <ScatterChart {...commonProps}>
            {chartConfig.showGrid && <CartesianGrid />}
            <XAxis dataKey={chartConfig.xKey} />
            <YAxis dataKey={chartConfig.yKeys?.[0]} />
            {chartConfig.showTooltip && <Tooltip cursor={{ strokeDasharray: '3 3' }} />}
            {chartConfig.showLegend && <Legend />}
            <Scatter
              dataKey={chartConfig.yKeys?.[0]}
              fill={chartConfig.colors?.[0]}
            />
          </ScatterChart>
        );

      default:
        return <div>Unsupported chart type: {displayType}</div>;
    }
  };

  // Chart type icons
  const getChartIcon = (type: ChartType) => {
    switch (type) {
      case 'line': return <LineChartIcon className="h-4 w-4" />;
      case 'bar': return <BarChart3 className="h-4 w-4" />;
      case 'area': return <TrendingUp className="h-4 w-4" />;
      case 'pie': return <PieChartIcon className="h-4 w-4" />;
      case 'scatter': return <BarChart3 className="h-4 w-4" />;
      default: return <BarChart3 className="h-4 w-4" />;
    }
  };

  if (toolCallState?.status === 'completed') {
    return (
      <Card className="w-full">
        <CardHeader>
          <div className="flex items-center justify-between">
            <div>
              <CardTitle className="flex items-center gap-2">
                {getChartIcon(displayType)}
                {chartConfig.title}
              </CardTitle>
              {chartConfig.description && (
                <CardDescription>{chartConfig.description}</CardDescription>
              )}
            </div>
            <Badge variant="default">Completed</Badge>
          </div>
        </CardHeader>
        <CardContent>
          <div className="text-sm text-muted-foreground mb-4">
            Chart rendered successfully with {chartConfig.data.length} data points
          </div>
        </CardContent>
      </Card>
    );
  }

  return (
    <Card className="w-full">
      <CardHeader>
        <div className="flex items-center justify-between">
          <div>
            <CardTitle className="flex items-center gap-2">
              {getChartIcon(displayType)}
              {chartConfig.title}
            </CardTitle>
            {chartConfig.description && (
              <CardDescription>{chartConfig.description}</CardDescription>
            )}
          </div>
          <div className="flex items-center gap-2">
            <Select value={displayType} onValueChange={(value: ChartType) => setCurrentChartType(value)}>
              <SelectTrigger className="w-32">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="line">Line</SelectItem>
                <SelectItem value="bar">Bar</SelectItem>
                <SelectItem value="area">Area</SelectItem>
                <SelectItem value="pie">Pie</SelectItem>
                <SelectItem value="scatter">Scatter</SelectItem>
              </SelectContent>
            </Select>
          </div>
        </div>
      </CardHeader>

      <CardContent>
        {/* Chart metadata */}
        <div className="flex gap-2 mb-4 text-xs text-muted-foreground">
          <Badge variant="secondary">
            Data Points: {chartConfig.data.length}
          </Badge>
          {chartConfig.xKey && (
            <Badge variant="outline">
              X: {chartConfig.xKey}
            </Badge>
          )}
          {chartConfig.yKeys?.map(key => (
            <Badge key={key} variant="outline">
              Y: {key}
            </Badge>
          ))}
        </div>

        {/* Chart container */}
        <div className="w-full" style={{ height: chartConfig.height }}>
          <ResponsiveContainer width="100%" height="100%">
            {renderChart()}
          </ResponsiveContainer>
        </div>

        {/* Actions */}
        <div className="flex justify-end mt-4">
          <Button onClick={handleComplete} size="sm">
            Complete
          </Button>
        </div>
      </CardContent>
    </Card>
  );
};

// Create DynamicChart tool
export const DynamicChartTool: DistriUiTool = {
  name: 'dynamic_chart',
  type: 'ui',
  autoExecute: false,
  description: 'Create dynamic charts from spreadsheet data',
  parameters: {
    type: 'object',
    properties: {
      data: {
        type: 'array',
        description: 'Chart data'
      },
      chartType: {
        type: 'string',
        description: 'Type of chart to create'
      },
      title: {
        type: 'string',
        description: 'Chart title'
      }
    }
  },
  component: (props: UiToolProps) => <DynamicChart {...props} />
}
