// IndexedDB utility for storing spreadsheet data

import { columnToLetter } from "@/utils/sheet";
import { GoogleSpreadsheetCell } from "google-spreadsheet";

// Define our own cell interface that matches what we need and is cloneable
export interface SpreadsheetCell {
  value: any;
  rowIndex: number;
  columnIndex: number;
  a1Row: number;
  a1Column: string;
  address: string;
  valueType: string | null;
  isFormula: boolean;
  calculatedValue?: string | null;
  formattedValue: string | null;
  formula: string | null;
  errorValue?: {
    type: string;
    message: string;
  };
  numberValue?: number;
  boolValue?: boolean;
  stringValue?: string;
  hyperlink?: string;
  note: string;
  userEnteredFormat?: any;
  effectiveFormat?: any;
}

export function createEmptyCell(rowIndex: number, columnIndex: number): SpreadsheetCell {
  return {
    value: '',
    rowIndex: rowIndex,
    columnIndex: columnIndex,
    a1Row: rowIndex,
    a1Column: columnToLetter(columnIndex),
    address: `${columnToLetter(columnIndex)}${rowIndex + 1}`,
    valueType: 'TEXT',
    isFormula: false,
    calculatedValue: null,
    formattedValue: null,
    formula: null,
    note: '',
    userEnteredFormat: {},
    effectiveFormat: {},
  };
}

// Helper function to convert GoogleSpreadsheetCell to our cloneable format
export const cellToCloneable = (cell: GoogleSpreadsheetCell): SpreadsheetCell => {
  return {
    value: cell.value,
    rowIndex: cell.rowIndex,
    columnIndex: cell.columnIndex,
    a1Row: cell.a1Row,
    a1Column: cell.a1Column,
    address: cell.a1Address,
    valueType: cell.valueType,
    isFormula: !!cell.formula,
    calculatedValue: cell.formattedValue,
    formattedValue: cell.formattedValue,
    formula: cell.formula,
    errorValue: cell.errorValue ? {
      type: cell.errorValue.type,
      message: cell.errorValue.message
    } : undefined,
    numberValue: cell.numberValue,
    boolValue: cell.boolValue,
    stringValue: cell.stringValue,
    hyperlink: cell.hyperlink,
    note: cell.note,
    userEnteredFormat: cell.userEnteredFormat,
    effectiveFormat: cell.effectiveFormat,
  };
};

// Helper function to convert our cloneable format back to a cell-like object
export const cloneableToCell = (cellData: SpreadsheetCell): SpreadsheetCell => {
  return cellData; // Already in the right format
};

// Extend the library types for our storage needs
export interface SheetMetadata {
  spreadsheetId: string;
  lastSynced: string;
  lastModified: string;
  sheets: Array<{
    properties: {
      sheetId: number;
      title: string;
      index: number;
      sheetType: string;
      gridProperties: {
        rowCount: number;
        columnCount: number;
      };
    };
  }>;
}

export interface SheetData {
  spreadsheetId: string;
  sheetId: number;
  title: string;
  cellIndex: Record<string, SpreadsheetCell>; // Use our own cell type
  maxRow: number;
  maxCol: number;
  lastUpdated: string;
}

class DatabaseManager {
  private dbName = 'SpreadsheetStore';
  private version = 1;
  private db: IDBDatabase | null = null;

  async init(): Promise<void> {
    return new Promise((resolve, reject) => {
      const request = indexedDB.open(this.dbName, this.version);

      request.onerror = () => reject(request.error);
      request.onsuccess = () => {
        this.db = request.result;
        resolve();
      };

      request.onupgradeneeded = (event) => {
        const db = (event.target as IDBOpenDBRequest).result;

        // Create metadata store
        if (!db.objectStoreNames.contains('metadata')) {
          const metadataStore = db.createObjectStore('metadata', { keyPath: 'spreadsheetId' });
          metadataStore.createIndex('lastSynced', 'lastSynced', { unique: false });
        }

        // Create sheet data store
        if (!db.objectStoreNames.contains('sheetData')) {
          const sheetDataStore = db.createObjectStore('sheetData', { keyPath: ['spreadsheetId', 'sheetId'] });
          sheetDataStore.createIndex('spreadsheetId', 'spreadsheetId', { unique: false });
          sheetDataStore.createIndex('lastUpdated', 'lastUpdated', { unique: false });
        }
      };
    });
  }

  async storeMetadata(metadata: SheetMetadata): Promise<void> {
    if (!this.db) await this.init();

    return new Promise((resolve, reject) => {
      const transaction = this.db!.transaction(['metadata'], 'readwrite');
      const store = transaction.objectStore('metadata');
      const request = store.put(metadata);

      request.onerror = () => reject(request.error);
      request.onsuccess = () => resolve();
    });
  }

  async getMetadata(spreadsheetId: string): Promise<SheetMetadata | null> {
    if (!this.db) await this.init();

    return new Promise((resolve, reject) => {
      const transaction = this.db!.transaction(['metadata'], 'readonly');
      const store = transaction.objectStore('metadata');
      const request = store.get(spreadsheetId);

      request.onerror = () => reject(request.error);
      request.onsuccess = () => resolve(request.result || null);
    });
  }

  async storeSheetData(sheetData: SheetData): Promise<void> {
    if (!this.db) await this.init();

    return new Promise((resolve, reject) => {
      const transaction = this.db!.transaction(['sheetData'], 'readwrite');
      const store = transaction.objectStore('sheetData');
      const request = store.put(sheetData);

      request.onerror = () => reject(request.error);
      request.onsuccess = () => resolve();
    });
  }

  async getSheetData(spreadsheetId: string, sheetId: number): Promise<SheetData | null> {
    if (!this.db) await this.init();

    return new Promise((resolve, reject) => {
      const transaction = this.db!.transaction(['sheetData'], 'readonly');
      const store = transaction.objectStore('sheetData');
      const request = store.get([spreadsheetId, sheetId]);

      request.onerror = () => reject(request.error);
      request.onsuccess = () => resolve(request.result || null);
    });
  }

  async getAllSheetData(spreadsheetId: string): Promise<SheetData[]> {
    if (!this.db) await this.init();

    return new Promise((resolve, reject) => {
      const transaction = this.db!.transaction(['sheetData'], 'readonly');
      const store = transaction.objectStore('sheetData');
      const index = store.index('spreadsheetId');
      const request = index.getAll(spreadsheetId);

      request.onerror = () => reject(request.error);
      request.onsuccess = () => resolve(request.result || []);
    });
  }

  async clearSpreadsheetData(spreadsheetId: string): Promise<void> {
    if (!this.db) await this.init();

    return new Promise((resolve, reject) => {
      const transaction = this.db!.transaction(['metadata', 'sheetData'], 'readwrite');

      // Clear metadata
      const metadataStore = transaction.objectStore('metadata');
      metadataStore.delete(spreadsheetId);

      // Clear sheet data
      const sheetDataStore = transaction.objectStore('sheetData');
      const index = sheetDataStore.index('spreadsheetId');
      const sheetDataRequest = index.openCursor(spreadsheetId);

      sheetDataRequest.onsuccess = (event) => {
        const cursor = (event.target as IDBRequest).result;
        if (cursor) {
          cursor.delete();
          cursor.continue();
        }
      };

      transaction.oncomplete = () => resolve();
      transaction.onerror = () => reject(transaction.error);
    });
  }
}

export const dbManager = new DatabaseManager(); 
