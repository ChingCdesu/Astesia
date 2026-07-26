import { cn } from '@/lib/utils';
import { DbType } from '@/types/database';
import { SiClickhouse, SiSqlite } from 'react-icons/si';
import { DiMysql, DiMsqlServer, DiRedis, DiPostgresql, DiMongodb } from 'react-icons/di';
import type { IconType } from 'react-icons';

interface DbIconProps {
  dbType: DbType;
  className?: string;
  size?: number;
}

const iconMap: Record<DbType, { icon: IconType; color: string }> = {
  mysql: { icon: DiMysql, color: '#4479A1' },
  postgresql: { icon: DiPostgresql, color: '#4169E1' },
  sqlite: { icon: SiSqlite, color: '#003B57' },
  sqlserver: { icon: DiMsqlServer, color: '#CC2927' },
  mongodb: { icon: DiMongodb, color: '#47A248' },
  redis: { icon: DiRedis, color: '#DC382D' },
  clickhouse: { icon: SiClickhouse, color: '#FFCC01' },
};

export function DbIcon({ dbType, className, size = 16 }: DbIconProps) {
  const entry = iconMap[dbType];
  if (!entry) return null;
  const Icon = entry.icon;
  return <Icon size={size} color={entry.color} className={cn('shrink-0', className)} />;
}
