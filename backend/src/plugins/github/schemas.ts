import z from 'zod'

export const updatePullRequestCommentBodySchema = z.object({
  body: z.string().trim().min(1, 'Missing comment body'),
})

export const issueCommentBodySchema = z.object({
  body: z.string().trim().min(1, 'Missing comment body'),
})

export const updateDescriptionBodySchema = z.object({
  body: z.string().transform(value => value.trim()),
})

export const createPullRequestLineCommentBodySchema = z.object({
  body: z.string().trim().min(1, 'Missing comment body'),
  path: z.string().trim().min(1, 'Missing comment path'),
  commitId: z.string().trim().min(1, 'Missing comment commit id'),
  line: z.number().int().positive(),
  side: z.enum(['LEFT', 'RIGHT']),
  startLine: z.number().int().positive().optional(),
  startSide: z.enum(['LEFT', 'RIGHT']).optional(),
})

export const createPullRequestThreadReplyBodySchema = z.object({
  body: z.string().trim().min(1, 'Missing comment body'),
})

export const createPullRequestReviewBodySchema = z.object({
  event: z.enum(['COMMENT', 'APPROVE', 'REQUEST_CHANGES']),
  body: z.string().optional(),
})
