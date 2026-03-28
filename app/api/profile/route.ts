import { NextRequest, NextResponse } from 'next/server';
import { prisma } from '@/lib/prisma';
import { verifyToken, extractToken } from '@/lib/auth';
import { z } from 'zod';

const profileSchema = z.object({
  username: z.string().min(3).max(30).regex(/^[a-zA-Z0-9]+$/, 'Username must be alphanumeric only'),
  email: z.string().email('Invalid email format'),
});

export async function PUT(request: NextRequest) {
  try {
    const token = extractToken(request.headers.get('authorization'));
    if (!token) return NextResponse.json({ error: 'Unauthorized' }, { status: 401 });

    const payload = verifyToken(token);
    if (!payload) return NextResponse.json({ error: 'Invalid or expired token' }, { status: 401 });

    const body = await request.json();
    
    // Server-side validation with Zod
    const result = profileSchema.safeParse(body);
    if (!result.success) {
      return NextResponse.json({ 
        error: 'Validation failed', 
        details: result.error.flatten().fieldErrors 
      }, { status: 400 });
    }

    const { username, email } = result.data;

    // Check if username is already taken by another user
    if (username) {
      const existingUser = await prisma.user.findFirst({
        where: { 
          username,
          NOT: { id: payload.userId }
        }
      });
      if (existingUser) {
        return NextResponse.json({ error: 'Username is already taken' }, { status: 400 });
      }
    }

    // Check if email is already taken by another user
    const existingEmail = await prisma.user.findFirst({
      where: { 
        email,
        NOT: { id: payload.userId }
      }
    });
    if (existingEmail) {
      return NextResponse.json({ error: 'Email is already taken' }, { status: 400 });
    }

    const updatedUser = await prisma.user.update({
      where: { id: payload.userId },
      data: { username, email },
      select: {
        id: true,
        email: true,
        username: true,
        firstName: true,
        lastName: true,
      }
    });

    return NextResponse.json({ 
      success: true, 
      message: 'Profile updated successfully',
      user: updatedUser 
    });

  } catch (error) {
    console.error('Update profile error:', error);
    return NextResponse.json({ error: 'Internal server error' }, { status: 500 });
  }
}

export async function GET(request: NextRequest) {
  try {
    const token = extractToken(request.headers.get('authorization'));
    if (!token) return NextResponse.json({ error: 'Unauthorized' }, { status: 401 });

    const payload = verifyToken(token);
    if (!payload) return NextResponse.json({ error: 'Invalid or expired token' }, { status: 401 });

    const user = await prisma.user.findUnique({
      where: { id: payload.userId },
      select: {
        id: true,
        email: true,
        username: true,
        firstName: true,
        lastName: true,
      }
    });

    if (!user) return NextResponse.json({ error: 'User not found' }, { status: 404 });

    return NextResponse.json({ success: true, user });
  } catch (error) {
    console.error('Get profile error:', error);
    return NextResponse.json({ error: 'Internal server error' }, { status: 500 });
  }
}
