import { fixture, catalogRow } from '../test/fixtures.mjs';
import { editDaemon } from '../test/edit-peer.mjs';
export async function editFixtureTransport() {
  const f = fixture();
  f.catalog.value.items = ['arbitrary_vendor','future_actor'].map(actor_type => catalogRow(actor_type, 1n, {
    description:'Development fixture registration',creatable:true,template_config:{text:'Example',count:1n},
    in_ports:[{id:'input',primary:true}],out_ports:[{id:'event',primary:true}]}));
  f.snapshot.value.commands[0].declaration.config = {text:'Edit this field',count:1n};
  return editDaemon({fixtureValue:f}).transport;
}
