import { TrendingUp, TrendingDown, Minus } from 'lucide-react';
import { cn } from '@/lib/utils';

interface MetricCardProps {
  label: string;
  value: string | number;
  unit?: string;
  trend?: 'up' | 'down' | 'stable';
  className?: string;
}

export default function MetricCard({ label, value, unit, trend, className }: MetricCardProps) {
  return (
    <div
      className={cn(
        'rounded-lg border bg-card p-4 shadow-sm transition-colors',
        className
      )}
    >
      <p className="text-xs text-muted-foreground">{label}</p>
      <div className="mt-1 flex items-baseline gap-1">
        <span className="text-2xl font-bold tracking-tight">{value}</span>
        {unit && <span className="text-xs text-muted-foreground">{unit}</span>}
        {trend && (
          <span className="ml-auto">
            {trend === 'up' && (
              <TrendingUp className="h-4 w-4 text-green-500" />
            )}
            {trend === 'down' && (
              <TrendingDown className="h-4 w-4 text-red-500" />
            )}
            {trend === 'stable' && (
              <Minus className="h-4 w-4 text-muted-foreground" />
            )}
          </span>
        )}
      </div>
    </div>
  );
}
