export type AppErrorCode = 'not-connected' | 'not-supported' | 'storage' | 'validation' | 'cancelled' | 'internal';
export interface AppError {
  code: AppErrorCode;
  message: string;
  retryable: boolean;
}

export function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'object' && error && 'message' in error && typeof error.message === 'string') return error.message;
  return '操作未完成，请重试。';
}
