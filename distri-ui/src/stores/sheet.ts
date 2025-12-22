import { create } from "zustand";
import { GoogleSpreadsheet, GoogleSpreadsheetCell } from 'google-spreadsheet';
import { dbManager, SheetMetadata, SheetData, SpreadsheetCell, cellToCloneable, createEmptyCell } from "@/lib/database";




interface DeltaOperation {
  type: 'addRow' | 'addColumn'
  sheetId: number
  index: number
  count: number
}

interface SpreadsheetStore {
  spreadsheetId: string;
  isSyncing: boolean;
  error: string | null;
  metadata: SheetMetadata | null;
  sheetData: Record<number, SheetData>;
  syncProgress: { current: number; total: number; message: string } | null;
  deltas: DeltaOperation[];

  // Actions
  setSpreadsheetId: (id: string) => void;
  setIsSyncing: (loading: boolean) => void;
  setError: (error: string | null) => void;
  setSyncProgress: (progress: { current: number; total: number; message: string } | null) => void;
  sync: (driveToken: string) => Promise<void>;
  addRows: (sheetId: number, count?: number, index?: number) => void;
  addColumns: (sheetId: number, count?: number, index?: number) => void;
  clearDeltas: () => void;
  get_values: (sheetId?: number) => any[][];
  get_cell: (sheetId: number, row: number, col: number) => SpreadsheetCell | null;
  get_range: (sheetId: number, startRow: number, startCol: number, endRow: number, endCol: number) => SpreadsheetCell[][];
  get_sheet_info: (sheetId?: number) => { maxRow: number; maxCol: number; title: string } | null;
  getLastModifiedTime: () => string | null;
  getModifiedTime: (driveToken: string) => Promise<string | null>;
  clearData: () => Promise<void>;
}

export const useSpreadsheetStore = create<SpreadsheetStore>((set, get) => ({
  spreadsheetId: '',
  isSyncing: false,
  error: null,
  metadata: null,
  sheetData: {},
  syncProgress: null,
  deltas: [],

  setSpreadsheetId: (id: string) => {
    set({ spreadsheetId: id });
  },

  setIsSyncing: (loading: boolean) => {
    set({ isSyncing: loading });
  },

  setError: (error: string | null) => {
    set({ error });
  },

  setSyncProgress: (progress: { current: number; total: number; message: string } | null) => {
    set({ syncProgress: progress });
  },

  sync: async (driveToken: string) => {
    const { spreadsheetId } = get();
    if (!spreadsheetId) {
      set({ error: 'No spreadsheet ID provided' });
      return;
    }

    set({ isSyncing: true, error: null, syncProgress: { current: 0, total: 1, message: 'Initializing sync...' } });

    try {
      // Step 1: Initialize google-spreadsheet library
      get().setSyncProgress({ current: 1, total: 3, message: 'Connecting to spreadsheet...' });

      const doc = new GoogleSpreadsheet(spreadsheetId, { token: driveToken });

      // Load document properties
      await doc.loadInfo();

      // Get last modified time
      const lastModified = new Date().toISOString();

      const metadata: SheetMetadata = {
        spreadsheetId,
        lastSynced: new Date().toISOString(),
        lastModified,
        sheets: doc.sheetsByIndex.map((sheet, index) => ({
          properties: {
            sheetId: sheet.sheetId || index,
            title: sheet.title,
            index: index,
            sheetType: 'GRID',
            gridProperties: {
              rowCount: sheet.rowCount || 1000,
              columnCount: sheet.columnCount || 26,
            },
          },
        })),
      };

      console.log(metadata);


      // Store metadata in IndexedDB
      await dbManager.storeMetadata(metadata);

      // Step 2: Load all sheet data
      get().setSyncProgress({ current: 2, total: 3, message: 'Loading spreadsheet data...' });

      const sheetData: Record<number, SheetData> = {};

      // Process each sheet
      for (let sheetIndex = 0; sheetIndex < doc.sheetsByIndex.length; sheetIndex++) {
        const sheet = doc.sheetsByIndex[sheetIndex];
        const sheetId = sheet.sheetId || sheetIndex;
        const sheetTitle = sheet.title;

        console.log('Title', sheet.title);
        console.log('Row Count', sheet.rowCount);
        console.log('Column Count', sheet.columnCount);

        try {
          get().setSyncProgress({
            current: 2,
            total: 3,
            message: `Loading sheet "${sheetTitle}"...`
          });




          // Load all cells from the sheet
          await sheet.loadCells(`A1:Z${sheet.rowCount + 1}`);
          console.log(sheet.cellStats);

          // Create cell index for fast access
          const cellIndex: Record<string, SpreadsheetCell> = {};
          // Process all cells - use the cells property if available


          for (let rowIndex = 0; rowIndex < sheet.rowCount; rowIndex++) {
            for (let colIndex = 0; colIndex < sheet.columnCount; colIndex++) {
              console.log("cell", rowIndex, colIndex);
              const cell: GoogleSpreadsheetCell = sheet.getCell(rowIndex, colIndex);
              console.log("cell", cell);

              if (cell) {
                const rowIndex = cell.rowIndex;
                const colIndex = cell.columnIndex;
                cellIndex[`${rowIndex}:${colIndex}`] = cellToCloneable(cell);
              }
            }
          }

          const sheetDataItem: SheetData = {
            spreadsheetId,
            sheetId: sheetId,
            title: sheetTitle,
            cellIndex,
            maxRow: sheet.rowCount - 1,
            maxCol: sheet.columnCount - 1,
            lastUpdated: new Date().toISOString(),
          };

          // Store in IndexedDB
          await dbManager.storeSheetData(sheetDataItem);
          sheetData[sheetId] = sheetDataItem;
        } catch (error) {
          console.error(`Error loading sheet ${sheetTitle}:`, error);
        }
      }

      set({
        metadata,
        sheetData,
        isSyncing: false,
        error: null,
        syncProgress: null
      });

    } catch (error) {
      console.error('Error syncing spreadsheet:', error);
      set({
        isSyncing: false,
        error: error instanceof Error ? error.message : 'Unknown error occurred',
        syncProgress: null
      });
    }
  },

  addRows: (sheetId: number, count = 1, index?: number) => {
    const { sheetData, deltas } = get();
    const sheet = sheetData[sheetId];
    if (!sheet) return;

    const insertIndex = index ?? sheet.maxRow + 1;

    // Shift existing rows downwards
    for (let row = sheet.maxRow; row >= insertIndex; row--) {
      for (let col = 0; col <= sheet.maxCol; col++) {
        const oldKey = `${row}:${col}`;
        const newKey = `${row + count}:${col}`;
        sheet.cellIndex[newKey] = sheet.cellIndex[oldKey];
        delete sheet.cellIndex[oldKey];
      }
    }

    // Fill new rows with empty values
    for (let row = insertIndex; row < insertIndex + count; row++) {
      for (let col = 0; col <= sheet.maxCol; col++) {
        sheet.cellIndex[`${row}:${col}`] = createEmptyCell(row, col);
      }
    }

    sheet.maxRow += count;
    set({
      sheetData: { ...sheetData, [sheetId]: sheet },
      deltas: [...deltas, { type: 'addRow', sheetId, index: insertIndex, count }]
    });
  },

  addColumns: (sheetId: number, count = 1, index?: number) => {
    const { sheetData, deltas } = get();
    const sheet = sheetData[sheetId];
    if (!sheet) return;

    const insertIndex = index ?? sheet.maxCol + 1;

    // Shift existing columns to the right
    for (let col = sheet.maxCol; col >= insertIndex; col--) {
      for (let row = 0; row <= sheet.maxRow; row++) {
        const oldKey = `${row}:${col}`;
        const newKey = `${row}:${col + count}`;
        sheet.cellIndex[newKey] = sheet.cellIndex[oldKey];
        delete sheet.cellIndex[oldKey];
      }
    }

    // Fill new columns with empty values
    for (let col = insertIndex; col < insertIndex + count; col++) {
      for (let row = 0; row <= sheet.maxRow; row++) {
        sheet.cellIndex[`${row}:${col}`] = createEmptyCell(row, col);
      }
    }

    sheet.maxCol += count;
    set({
      sheetData: { ...sheetData, [sheetId]: sheet },
      deltas: [...deltas, { type: 'addColumn', sheetId, index: insertIndex, count }]
    });
  },

  clearDeltas: () => {
    set({ deltas: [] });
  },

  get_values: (sheetId?: number) => {
    const { sheetData, metadata } = get();

    if (!metadata || !metadata.sheets.length) {
      return [];
    }

    // If no sheetId specified, use the first sheet's actual sheetId
    const targetSheetId = sheetId ?? metadata.sheets[0].properties.sheetId;

    const sheet = sheetData[targetSheetId];
    if (!sheet) {
      console.warn(`Sheet with ID ${targetSheetId} not found in sheetData. Available keys:`, Object.keys(sheetData));
      return [];
    }

    // Convert cell index back to 2D array for compatibility
    const values: any[][] = [];
    for (let row = 0; row <= sheet.maxRow; row++) {
      const rowData: any[] = [];
      for (let col = 0; col <= sheet.maxCol; col++) {
        const cellKey = `${row}:${col}`;
        const cell = sheet.cellIndex[cellKey];
        rowData.push(cell?.value || '');
      }
      values.push(rowData);
    }
    return values;
  },

  get_sheet_info: (sheetId?: number) => {
    const { sheetData, metadata } = get();

    if (!metadata || !metadata.sheets.length) {
      return null;
    }

    const targetSheetId = sheetId ?? metadata.sheets[0].properties.sheetId;
    const sheet = sheetData[targetSheetId];

    if (!sheet) return null;

    return {
      maxRow: sheet.maxRow,
      maxCol: sheet.maxCol,
      title: sheet.title
    };
  },

  get_cell: (sheetId: number, row: number, col: number) => {
    const { sheetData } = get();
    const sheet = sheetData[sheetId];

    if (!sheet) return null;

    // Use cell index for fast access
    const cellKey = `${row}:${col}`;
    return sheet.cellIndex[cellKey] || null;
  },

  get_range: (sheetId: number, startRow: number, startCol: number, endRow: number, endCol: number) => {
    const { sheetData } = get();
    const sheet = sheetData[sheetId];

    if (!sheet) return [];

    const range: SpreadsheetCell[][] = [];

    for (let row = startRow; row <= endRow; row++) {
      const rowData: SpreadsheetCell[] = [];
      for (let col = startCol; col <= endCol; col++) {
        const cellKey = `${row}:${col}`;
        const cell = sheet.cellIndex[cellKey];
        rowData.push(cell || null as any);
      }
      range.push(rowData);
    }

    return range;
  },

  clearData: async () => {
    const { spreadsheetId } = get();
    if (!spreadsheetId) return;

    try {
      // Clear from IndexedDB only
      await dbManager.clearSpreadsheetData(spreadsheetId);

      set({ metadata: null, sheetData: {}, syncProgress: null });
    } catch (error) {
      console.error('Error clearing data:', error);
      set({ error: 'Failed to clear data' });
    }
  },

  getLastModifiedTime: () => {
    const { metadata } = get();
    return metadata?.lastModified || null;
  },

  getModifiedTime: async (driveToken: string) => {
    const { spreadsheetId } = get();
    if (!spreadsheetId) return null;

    try {
      // Use google-spreadsheet library to get modified time
      const doc = new GoogleSpreadsheet(spreadsheetId, { token: driveToken });

      await doc.loadInfo();

      // The library doesn't directly expose modified time, so we'll use current time
      // In a real implementation, you might want to use the Drive API for this
      return new Date().toISOString();
    } catch (error) {
      console.error('Error getting modified time:', error);
      // Fallback to cached value
      const { metadata } = get();
      return metadata?.lastModified || null;
    }
  },
}));

// Add a method to load cached data
export const loadCachedData = async (spreadsheetId: string) => {
  try {
    // Set the spreadsheet ID first
    useSpreadsheetStore.setState({ spreadsheetId });

    // Load metadata from IndexedDB only
    const metadata = await dbManager.getMetadata(spreadsheetId);

    if (metadata) {
      // Load sheet data from IndexedDB
      const sheetDataArray = await dbManager.getAllSheetData(spreadsheetId);
      const sheetData: Record<number, SheetData> = {};

      sheetDataArray.forEach(data => {
        sheetData[data.sheetId] = data;
      });

      console.log('Loaded cached data:', {
        spreadsheetId,
        metadataSheets: metadata.sheets.map(s => ({ id: s.properties.sheetId, title: s.properties.title })),
        sheetDataKeys: Object.keys(sheetData),
        sheetDataTitles: Object.values(sheetData).map(s => ({ id: s.sheetId, title: s.title }))
      });

      useSpreadsheetStore.setState({ metadata, sheetData });
      return true;
    } else {
      // No cached data found
    }
    return false;
  } catch (error) {
    console.error('Error loading stored data:', error);
    return false;
  } finally {
    useSpreadsheetStore.setState({ isSyncing: false })
  }
};