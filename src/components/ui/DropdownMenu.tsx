import { forwardRef, type ComponentPropsWithoutRef, type ComponentRef } from 'react';
import * as Menu from '@radix-ui/react-dropdown-menu';
import { cn } from '../../lib/cn';

export const DropdownMenu = Menu.Root;
export const DropdownMenuTrigger = Menu.Trigger;
export const DropdownMenuSub = Menu.Sub;

export const DropdownMenuContent = forwardRef<ComponentRef<typeof Menu.Content>, ComponentPropsWithoutRef<typeof Menu.Content>>(
  function DropdownMenuContent({ className, sideOffset = 6, collisionPadding = 8, ...props }, ref) {
    return <Menu.Portal><Menu.Content ref={ref} sideOffset={sideOffset} collisionPadding={collisionPadding} className={cn('action-menu', className)} {...props} /></Menu.Portal>;
  },
);

export const DropdownMenuItem = forwardRef<ComponentRef<typeof Menu.Item>, ComponentPropsWithoutRef<typeof Menu.Item>>(
  function DropdownMenuItem({ className, ...props }, ref) {
    return <Menu.Item ref={ref} className={cn('action-menu-item', className)} {...props} />;
  },
);

export const DropdownMenuSubTrigger = forwardRef<ComponentRef<typeof Menu.SubTrigger>, ComponentPropsWithoutRef<typeof Menu.SubTrigger>>(
  function DropdownMenuSubTrigger({ className, ...props }, ref) {
    return <Menu.SubTrigger ref={ref} className={cn('action-menu-item', className)} {...props} />;
  },
);

export const DropdownMenuSubContent = forwardRef<ComponentRef<typeof Menu.SubContent>, ComponentPropsWithoutRef<typeof Menu.SubContent>>(
  function DropdownMenuSubContent({ className, collisionPadding = 8, ...props }, ref) {
    return <Menu.Portal><Menu.SubContent ref={ref} collisionPadding={collisionPadding} className={cn('action-menu', className)} {...props} /></Menu.Portal>;
  },
);

export function DropdownMenuSeparator() {
  return <Menu.Separator className="action-menu-separator" />;
}
