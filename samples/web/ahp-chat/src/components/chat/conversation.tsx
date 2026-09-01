import { Bot, CircleAlert, UserRound } from "lucide-react"

import { ResponsePartKind, type URI } from "@microsoft/agent-host-protocol"

import type { ChatView } from "@/ahp/projection"
import type { ConversationPartView, ToolConfirmationOptionView } from "@/ahp/projection-types"
import { Bubble, BubbleContent } from "@/components/ui/bubble"
import {
  MessageScroller,
  MessageScrollerButton,
  MessageScrollerContent,
  MessageScrollerItem,
  MessageScrollerProvider,
  MessageScrollerViewport,
} from "@/components/ui/message-scroller"
import { Marker, MarkerContent, MarkerIcon } from "@/components/ui/marker"
import { Message, MessageAvatar, MessageContent, MessageHeader } from "@/components/ui/message"

import { ToolCallCard } from "./tool-call-card"

interface ConversationProps {
  chat: ChatView
  workspace?: URI
  onConfirmToolCall: (turnId: string, toolCallId: string, option: ToolConfirmationOptionView) => boolean
}

function ResponsePart({
  part,
  turnId,
  workspace,
  onConfirmToolCall,
}: {
  part: ConversationPartView
  turnId: string
  workspace?: URI
  onConfirmToolCall: ConversationProps["onConfirmToolCall"]
}) {
  switch (part.kind) {
    case ResponsePartKind.Markdown:
      return <Bubble variant="secondary"><BubbleContent className="whitespace-pre-wrap">{part.content}</BubbleContent></Bubble>
    case ResponsePartKind.Reasoning:
      return <Marker variant="border"><MarkerContent className="whitespace-pre-wrap text-muted-foreground">{part.content}</MarkerContent></Marker>
    case ResponsePartKind.SystemNotification:
      return <Marker variant="border"><MarkerContent>{part.content}</MarkerContent></Marker>
    case ResponsePartKind.ContentRef:
      return <Marker variant="border"><MarkerContent>Response resource: {part.uri}</MarkerContent></Marker>
    case ResponsePartKind.InputRequest:
      return <Marker variant="border"><MarkerContent>{part.message}</MarkerContent></Marker>
    case ResponsePartKind.ToolCall:
      return (
        <ToolCallCard
          key={`${part.tool.toolCallId}-${part.tool.status}`}
          tool={part.tool}
          workspace={workspace}
          onConfirm={(option) => onConfirmToolCall(turnId, part.tool.toolCallId, option)}
        />
      )
  }
}

function ConversationItem({
  turn,
  workspace,
  onConfirmToolCall,
}: {
  turn: ChatView["turns"][number]
  workspace?: URI
  onConfirmToolCall: ConversationProps["onConfirmToolCall"]
}) {
  return (
    <div className="space-y-3">
      <Message align="end">
        <MessageAvatar><UserRound className="size-4" /></MessageAvatar>
        <MessageContent>
          <Bubble align="end">
            <BubbleContent className="whitespace-pre-wrap">{turn.userText}</BubbleContent>
          </Bubble>
        </MessageContent>
      </Message>
      <Message>
        <MessageAvatar><Bot className="size-4" /></MessageAvatar>
        <MessageContent>
          <MessageHeader>Agent</MessageHeader>
          {turn.responseParts.length ? (
            <div className="space-y-2">
              {turn.responseParts.map((part, index) => (
                <ResponsePart
                  key={part.kind === ResponsePartKind.ToolCall ? part.tool.toolCallId : `${part.kind}-${index}`}
                  part={part}
                  turnId={turn.id}
                  workspace={workspace}
                  onConfirmToolCall={onConfirmToolCall}
                />
              ))}
            </div>
          ) : turn.state === "streaming" ? (
            <Bubble variant="secondary">
              <BubbleContent><span className="animate-pulse text-muted-foreground">Thinking…</span></BubbleContent>
            </Bubble>
          ) : null}
          {turn.error && (
            <Marker variant="border" className="text-destructive">
              <MarkerIcon><CircleAlert /></MarkerIcon>
              <MarkerContent>{turn.error}</MarkerContent>
            </Marker>
          )}
        </MessageContent>
      </Message>
    </div>
  )
}

export function Conversation({ chat, workspace, onConfirmToolCall }: ConversationProps) {
  const turns = chat.activeTurn ? [...chat.turns, chat.activeTurn] : chat.turns

  return (
    <MessageScrollerProvider autoScroll defaultScrollPosition="end">
      <MessageScroller className="min-h-0 flex-1">
        <MessageScrollerViewport aria-label={`${chat.title} conversation`}>
          <MessageScrollerContent className="gap-5 px-4 py-6 sm:px-8">
            <Marker variant="separator"><MarkerContent>{chat.title}</MarkerContent></Marker>
            {turns.length === 0 ? (
              <div className="grid min-h-48 place-items-center rounded-xl border border-dashed p-6 text-center text-sm text-muted-foreground">
                Send a prompt to start this conversation.
              </div>
            ) : (
              turns.map((turn) => (
                <MessageScrollerItem key={turn.id} messageId={turn.id} scrollAnchor={turn.state === "streaming"}>
                  <ConversationItem turn={turn} workspace={workspace} onConfirmToolCall={onConfirmToolCall} />
                </MessageScrollerItem>
              ))
            )}
          </MessageScrollerContent>
        </MessageScrollerViewport>
        <MessageScrollerButton />
      </MessageScroller>
    </MessageScrollerProvider>
  )
}
