export interface CellChange {
  rowIndex: number;
  columnName: string;
  oldValue: any;
  newValue: any;
}

export interface NewRow {
  values: Record<string, any>;
}
