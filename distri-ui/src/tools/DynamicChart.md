# DynamicChart Tool Documentation

The `DynamicChart` component is a powerful, flexible charting tool that can render various types of charts using recharts. It supports multiple data formats and provides interactive features for data visualization.

## Supported Chart Types

- **Line Chart** (`line`) - For time series and trend data
- **Bar Chart** (`bar`) - For categorical comparisons
- **Area Chart** (`area`) - For showing cumulative data or filled trends
- **Pie Chart** (`pie`) - For showing proportions and percentages
- **Scatter Chart** (`scatter`) - For correlation and distribution analysis

## Data Format

The tool accepts data in a flexible JSON format with the following structure:

```typescript
interface ChartConfig {
  type?: 'line' | 'bar' | 'area' | 'pie' | 'scatter';  // Default: 'line'
  title?: string;                                       // Chart title
  description?: string;                                 // Chart description
  data: Array<{[key: string]: string | number}>;      // Chart data points
  xKey?: string;                                       // X-axis key (auto-detected if not provided)
  yKeys?: string[];                                    // Y-axis keys (auto-detected if not provided)
  colors?: string[];                                   // Custom colors for series
  width?: number;                                      // Chart width
  height?: number;                                     // Chart height (default: 400)
  showGrid?: boolean;                                  // Show grid lines (default: true)
  showLegend?: boolean;                                // Show legend (default: true)
  showTooltip?: boolean;                               // Show tooltip (default: true)
}
```

## Auto-Detection Features

The component includes intelligent auto-detection:

1. **X-axis Key Detection**: Automatically finds keys named `time`, `date`, `timestamp`, `x`, `label`, `name`, or `category`, or defaults to the first key
2. **Y-axis Keys Detection**: Automatically uses all numeric keys except the x-axis key
3. **Data Type Inference**: Handles mixed string/number data appropriately

## Usage Examples

### Example 1: Simple Time Series Line Chart

```json
{
  "type": "line",
  "title": "Website Traffic Over Time",
  "description": "Daily page views for the last 7 days",
  "data": [
    {"date": "2024-01-01", "pageViews": 1200, "uniqueVisitors": 800},
    {"date": "2024-01-02", "pageViews": 1350, "uniqueVisitors": 900},
    {"date": "2024-01-03", "pageViews": 1100, "uniqueVisitors": 750},
    {"date": "2024-01-04", "pageViews": 1500, "uniqueVisitors": 950},
    {"date": "2024-01-05", "pageViews": 1800, "uniqueVisitors": 1100},
    {"date": "2024-01-06", "pageViews": 2000, "uniqueVisitors": 1200},
    {"date": "2024-01-07", "pageViews": 1900, "uniqueVisitors": 1150}
  ]
}
```

### Example 2: Sales Comparison Bar Chart

```json
{
  "type": "bar",
  "title": "Monthly Sales by Product",
  "description": "Product sales comparison for Q1 2024",
  "data": [
    {"product": "Product A", "sales": 25000, "profit": 8000},
    {"product": "Product B", "sales": 18000, "profit": 6200},
    {"product": "Product C", "sales": 32000, "profit": 12000},
    {"product": "Product D", "sales": 15000, "profit": 4500}
  ],
  "xKey": "product",
  "yKeys": ["sales", "profit"],
  "colors": ["#8884d8", "#82ca9d"]
}
```

### Example 3: Market Share Pie Chart

```json
{
  "type": "pie",
  "title": "Market Share Distribution",
  "description": "Current market share by company",
  "data": [
    {"company": "Company A", "marketShare": 35},
    {"company": "Company B", "marketShare": 28},
    {"company": "Company C", "marketShare": 20},
    {"company": "Company D", "marketShare": 12},
    {"company": "Others", "marketShare": 5}
  ],
  "xKey": "company",
  "yKeys": ["marketShare"]
}
```

### Example 4: Performance Metrics Area Chart

```json
{
  "type": "area",
  "title": "System Performance Metrics",
  "description": "CPU and Memory usage over the last hour",
  "data": [
    {"time": "14:00", "cpu": 45, "memory": 62, "disk": 78},
    {"time": "14:15", "cpu": 52, "memory": 68, "disk": 79},
    {"time": "14:30", "cpu": 48, "memory": 65, "disk": 82},
    {"time": "14:45", "cpu": 55, "memory": 72, "disk": 84},
    {"time": "15:00", "cpu": 42, "memory": 58, "disk": 81}
  ],
  "height": 350,
  "showGrid": true
}
```

### Example 5: Correlation Scatter Plot

```json
{
  "type": "scatter",
  "title": "Revenue vs Marketing Spend",
  "description": "Correlation between marketing investment and revenue",
  "data": [
    {"marketingSpend": 10000, "revenue": 85000},
    {"marketingSpend": 15000, "revenue": 120000},
    {"marketingSpend": 8000, "revenue": 75000},
    {"marketingSpend": 20000, "revenue": 150000},
    {"marketingSpend": 12000, "revenue": 95000},
    {"marketingSpend": 18000, "revenue": 140000}
  ],
  "xKey": "marketingSpend",
  "yKeys": ["revenue"]
}
```

## Advanced Configuration

### Custom Colors

```json
{
  "type": "line",
  "title": "Custom Styled Chart",
  "data": [...],
  "colors": ["#ff6b6b", "#4ecdc4", "#45b7d1", "#f9ca24", "#f0932b"]
}
```

### Minimal Configuration

The component can work with minimal configuration thanks to auto-detection:

```json
{
  "data": [
    {"month": "Jan", "sales": 4000},
    {"month": "Feb", "sales": 3000},
    {"month": "Mar", "sales": 5000}
  ]
}
```

This will automatically:
- Use "line" as the chart type
- Detect "month" as the x-axis
- Detect "sales" as the y-axis
- Generate a title "Dynamic Chart"

## Features

- **Interactive Chart Type Switching**: Users can change chart types using the dropdown
- **Responsive Design**: Charts automatically adjust to container size
- **Tooltip Support**: Hover tooltips show detailed data values
- **Legend Support**: Automatic legend generation for multi-series data
- **Grid Lines**: Optional grid lines for better readability
- **Custom Styling**: Support for custom colors and dimensions
- **Data Point Badges**: Shows metadata about the chart (data points, axes)
- **Completion Tracking**: Integrates with the DistriUI tool completion system

## Error Handling

- **Empty Data**: Shows a helpful "No data available" message with an icon
- **Invalid JSON**: Gracefully handles malformed input data
- **Missing Keys**: Auto-detects appropriate keys or falls back to defaults
- **Type Safety**: TypeScript interfaces ensure data integrity

## Best Practices for Agents

1. **Provide Clear Titles**: Always include a descriptive title and description
2. **Use Appropriate Chart Types**: 
   - Line charts for time series data
   - Bar charts for categorical comparisons
   - Pie charts for proportional data (ensure values represent parts of a whole)
   - Area charts for cumulative metrics
   - Scatter plots for correlation analysis

3. **Data Quality**: Ensure numeric data is properly formatted as numbers, not strings
4. **Key Naming**: Use descriptive key names (e.g., "revenue" instead of "val1")
5. **Color Coordination**: Use colors that are accessible and meaningful
6. **Appropriate Sizing**: Consider the amount of data when setting chart height

## Integration with DistriUI

The DynamicChart component integrates seamlessly with the DistriUI tool system:

- Receives data through the `toolCall.input` parameter
- Provides completion feedback through the `completeTool` callback
- Shows loading states and completion status
- Follows DistriUI design patterns and styling

To use this tool in your agent, simply call it with the appropriate data structure and the component will handle the visualization automatically.
