import { forwardRef, type ButtonHTMLAttributes } from 'react';
import { cn } from '../../lib/cn';

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: 'primary' | 'secondary' | 'ghost';
  iconOnly?: boolean;
}
export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button({ variant = 'secondary', iconOnly, className, type = 'button', ...props }, ref) {
  return <button ref={ref} type={type} className={cn('button', `button-${variant}`, iconOnly && 'icon-button', className)} {...props} />;
});
