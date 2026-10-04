// IndexedDB Persistent Progress Tracker for L++ Duolingo Academy

export interface UserProgress {
  id: string;
  xp: number;
  streak: number;
  lastActiveDate: string;
  completedLessons: string[];
  unlockedBadges: string[];
  hearts: number;
  codeSubmissions: Record<string, string>;
}

const DB_NAME = "LppAcademyDB";
const DB_VERSION = 1;
const STORE_NAME = "userProgress";

export function initDB(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, DB_VERSION);

    request.onupgradeneeded = (event: any) => {
      const db = event.target.result;
      if (!db.objectStoreNames.contains(STORE_NAME)) {
        db.createObjectStore(STORE_NAME, { keyPath: "id" });
      }
    };

    request.onsuccess = (event: any) => {
      resolve(event.target.result);
    };

    request.onerror = (event: any) => {
      reject(event.target.error);
    };
  });
}

export async function getUserProgress(): Promise<UserProgress> {
  const db = await initDB();
  return new Promise((resolve) => {
    const tx = db.transaction(STORE_NAME, "readonly");
    const store = tx.objectStore(STORE_NAME);
    const req = store.get("default_user");

    req.onsuccess = () => {
      if (req.result) {
        resolve(req.result);
      } else {
        const initial: UserProgress = {
          id: "default_user",
          xp: 0,
          streak: 1,
          lastActiveDate: new Date().toISOString().split("T")[0],
          completedLessons: [],
          unlockedBadges: ["novice"],
          hearts: 5,
          codeSubmissions: {},
        };
        saveUserProgress(initial);
        resolve(initial);
      }
    };
  });
}

export async function saveUserProgress(progress: UserProgress): Promise<void> {
  const db = await initDB();
  return new Promise((resolve, reject) => {
    const tx = db.transaction(STORE_NAME, "readwrite");
    const store = tx.objectStore(STORE_NAME);
    const req = store.put(progress);
    req.onsuccess = () => resolve();
    req.onerror = () => reject(req.error);
  });
}
