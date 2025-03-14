use async_trait::async_trait;
use crate::command::args::textcomponent::TextComponentArgConsumer;

use crate::command::args::players::PlayersArgumentConsumer;

use crate::command::args::{Arg, ConsumedArgs};
use crate::command::dispatcher::CommandError;
use crate::command::dispatcher::CommandError::InvalidConsumption;
use crate::command::tree::CommandTree;
use crate::command::tree::builder::argument;
use crate::command::{CommandExecutor, CommandSender};
use crate::server::Server;

const NAMES: [&str; 1] = ["tellraw"];

const DESCRIPTION: &str = "Send a raw message to a player.";

const ARG_MESSAGE: &str = "message";
const ARG_TARGET: &str = "target";

struct Executor;

#[async_trait]
impl CommandExecutor for Executor {
    async fn execute<'a>(
        &self,
        _sender: &mut CommandSender<'a>,
        _server: &Server,
        args: &ConsumedArgs<'a>,
    ) -> Result<(), CommandError> {
        let Some(Arg::Players(targets)) = args.get(ARG_TARGET) else {
            return Err(InvalidConsumption(Some(ARG_TARGET.into())));
        };
        let Some(Arg::TextComponent(message)) = args.get(&ARG_MESSAGE) else {
            return Err(InvalidConsumption(Some(ARG_MESSAGE.into())));
        };

        for target in targets {
            target.send_system_message_raw(message, false).await;
        }

        Ok(())
    }
}

pub fn init_command_tree() -> CommandTree {
    CommandTree::new(NAMES, DESCRIPTION).then(
        argument(ARG_TARGET, PlayersArgumentConsumer).then(
            argument(ARG_MESSAGE, TextComponentArgConsumer).execute(Executor)
        )
    )
}
