import { create } from "zustand";
import { ContextRange } from "@/types";

interface RangeContextStore {
  // State
  selectedRange: { startRow: number; startCol: number; endRow: number; endCol: number } | null;
  isSelecting: boolean;
  selectionType: 'input' | 'output' | null;
  contextRanges: ContextRange[];
  currentSheetId: number | null;
  currentSheetTitle: string | null;

  // Actions
  setSelectedRange: (range: { startRow: number; startCol: number; endRow: number; endCol: number } | null) => void;
  setIsSelecting: (selecting: boolean) => void;
  setSelectionType: (type: 'input' | 'output' | null) => void;
  setCurrentSheet: (sheetId: number, sheetTitle: string) => void;

  // Context range actions
  addContextRange: (contextRange: ContextRange) => void;
  removeContextRange: (id: string) => void;
  clearContextRanges: () => void;

  // Selection mode actions
  startSelection: (type: 'input' | 'output') => void;
  cancelSelection: () => void;

  // Utility actions
  clearAll: () => void;
}

export const useRangeContextStore = create<RangeContextStore>((set, get) => ({
  // Initial state
  selectedRange: null,
  isSelecting: false,
  selectionType: null,
  contextRanges: [],
  currentSheetId: null,
  currentSheetTitle: null,

  // Basic setters
  setSelectedRange: (range) => set({ selectedRange: range }),
  setIsSelecting: (selecting) => set({ isSelecting: selecting }),
  setSelectionType: (type) => set({ selectionType: type }),
  setCurrentSheet: (sheetId, sheetTitle) => {
    const { currentSheetId } = get();
    // Clear selectedRange when switching to a different sheet
    if (currentSheetId !== sheetId) {
      set({ currentSheetId: sheetId, currentSheetTitle: sheetTitle, selectedRange: null });
    } else {
      set({ currentSheetId: sheetId, currentSheetTitle: sheetTitle });
    }
  },

  // Context range actions
  addContextRange: (contextRange) => {
    const { contextRanges } = get();
    console.log('Store: addContextRange called with', contextRange)
    console.log('Store: current contextRanges', contextRanges)

    // Check if range already exists (including sheet information)
    const isAlreadyInContext = contextRanges.some(cr =>
      cr.range.startRow === contextRange.range.startRow &&
      cr.range.startCol === contextRange.range.startCol &&
      cr.range.endRow === contextRange.range.endRow &&
      cr.range.endCol === contextRange.range.endCol &&
      cr.sheetId === contextRange.sheetId
    );

    console.log('Store: isAlreadyInContext', isAlreadyInContext)

    if (!isAlreadyInContext) {
      const newContextRanges = [...contextRanges, contextRange]
      console.log('Store: setting new contextRanges', newContextRanges)
      set({ contextRanges: newContextRanges });
    } else {
      console.log('Store: range already exists, not adding')
    }
  },

  removeContextRange: (id) => {
    const { contextRanges, selectedRange } = get();
    console.log('Store: removeContextRange called with id', id)
    console.log('Store: current contextRanges', contextRanges)
    const filteredRanges = contextRanges.filter(cr => cr.id !== id)
    console.log('Store: filtered contextRanges', filteredRanges)

    // Check if the removed range matches the current selectedRange and clear it
    const removedRange = contextRanges.find(cr => cr.id === id)
    if (removedRange && selectedRange &&
      removedRange.range.startRow === selectedRange.startRow &&
      removedRange.range.startCol === selectedRange.startCol &&
      removedRange.range.endRow === selectedRange.endRow &&
      removedRange.range.endCol === selectedRange.endCol) {
      console.log('Store: clearing selectedRange because it matches removed range')
      set({ contextRanges: filteredRanges, selectedRange: null });
    } else {
      set({ contextRanges: filteredRanges });
    }
  },

  clearContextRanges: () => set({ contextRanges: [] }),

  // Selection mode actions
  startSelection: (type) => {
    set({
      isSelecting: true,
      selectionType: type,
    });
  },

  cancelSelection: () => {
    set({
      isSelecting: false,
      selectionType: null,
    });
  },

  // Utility actions
  clearAll: () => {
    set({
      selectedRange: null,
      isSelecting: false,
      selectionType: null,
      contextRanges: [],
      currentSheetId: null,
      currentSheetTitle: null,
    });
  }
})); 