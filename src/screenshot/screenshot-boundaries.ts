/** Used ONLY by the separate screenshot build. Server functions never enter its bundle. */
function unavailable(): never {
  throw new Error("Application operations are unavailable in a screenshot.");
}

export {
  unavailable as listAgents,
  unavailable as getAgent,
  unavailable as getAgentChat,
  unavailable as getAgentChatHistory,
  unavailable as getAgentDashboard,
  unavailable as getRiskSettings,
  unavailable as getTradingAccountChoices,
  unavailable as closeAgentPosition,
  unavailable as sendAgentMessage,
  unavailable as setAgentEnabled,
  unavailable as listSubagents,
  unavailable as getDashboardSubagents,
  unavailable as getSubagent,
  unavailable as getSubagentRun,
  unavailable as runSubagent,
  unavailable as saveSubagentSection,
  unavailable as stopSubagentRun,
  unavailable as getIndicator,
  unavailable as getIndicatorVersions,
  unavailable as listIndicators,
  unavailable as renderChartIndicators,
};

export const authClient = { signOut: unavailable };
