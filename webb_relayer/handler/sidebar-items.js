initSidebarItems({"enum":[["Command","Enumerates the supported commands for chain specific relayers"],["CommandResponse","Enumerates the command responses"],["CommandType","Enumerates the supported protocols for relaying transactions"],["NetworkStatus","Enumerates the network status response of the relayer"],["WithdrawStatus","Enumerates the withdraw status response of the relayer"]],"fn":[["accept_connection","Sets up a websocket connection."],["calculate_fee","Calculates the fee for a given transaction"],["handle_cmd","Handles the command prompts for EVM and Substrate chains"],["handle_evm","Handler for EVM commands"],["handle_ip_info","Handles the `ip` address response"],["handle_leaves_cache_evm","Handles leaf data requests for evm"],["handle_leaves_cache_substrate","Handles leaf data requests for substrate"],["handle_relayer_info","Handles relayer configuration requests"],["handle_socket_info","Handles the socket address response"],["handle_substrate","Handler for Substrate commands"],["handle_text","Sets up a websocket channels for message sending."],["into_withdraw_error",""]],"struct":[["IpInformationResponse","Representation for IP address response"]],"type":[["CommandStream","Type alias for mpsc::Sender"],["EvmCommand","The command type for EVM txes"],["SubstrateCommand","The command type for Substrate pallet txes"]]});