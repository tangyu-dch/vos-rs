#!/usr/bin/env python3
import asyncio
import json
import uuid
import sys
import argparse
from nats.aio.client import Client as NATS

# Color formatting for beautiful CLI logging
class colors:
    HEADER = '\033[95m'
    OKBLUE = '\033[94m'
    OKCYAN = '\033[96m'
    OKGREEN = '\033[92m'
    WARNING = '\033[93m'
    FAIL = '\033[91m'
    ENDC = '\033[0m'
    BOLD = '\033[1m'

import os
from datetime import datetime

os.makedirs("logs", exist_ok=True)
log_file = open("logs/real_vci_events.log", "a", encoding="utf-8")

# Write section header
log_file.write(f"\n--- TEST SESSION START: {datetime.now().isoformat()} ---\n")
log_file.flush()

def log_to_file(tag, msg, payload=None):
    timestamp = datetime.now().strftime("%Y-%m-%d %H:%M:%S.%f")[:-3]
    log_line = f"[{timestamp}] [{tag}] {msg}\n"
    log_file.write(log_line)
    if payload:
        log_file.write(f"Payload:\n{json.dumps(payload, indent=2, ensure_ascii=False)}\n\n")
    log_file.flush()

def log_info(msg, payload=None):
    print(f"{colors.OKBLUE}[INFO]{colors.ENDC} {msg}")
    log_to_file("INFO", msg, payload)

def log_success(msg, payload=None):
    print(f"{colors.OKGREEN}[SUCCESS]{colors.ENDC} {msg}")
    log_to_file("SUCCESS", msg, payload)

def log_warn(msg, payload=None):
    print(f"{colors.WARNING}[WARN]{colors.ENDC} {msg}")
    log_to_file("WARN", msg, payload)

def log_error(msg, payload=None):
    print(f"{colors.FAIL}[ERROR]{colors.ENDC} {msg}")
    log_to_file("ERROR", msg, payload)

async def run_inbound_dial_scenario(nc, nats_url, host_ip):
    """
    Scenario 1: 分机直拨被叫 (Inbound Call with Automatic Dialing)
    Listens for 'call_initiated' and replies with 'dial' instruction.
    """
    log_info(f"Starting Scenario 1: Inbound Call with Automatic Dialing (using host IP: {host_ip})...")

    async def message_handler(msg):
        subject = msg.subject
        reply = msg.reply
        data = json.loads(msg.data.decode())
        
        event_type = data.get("event_type")
        call_id = data.get("call_id")
        event_data = data.get("data", {})
        leg = event_data.get("leg")

        log_info(f"Received Event: {colors.BOLD}{event_type}{colors.ENDC} | Call-ID: {call_id} | Leg: {leg}")

        # Always reply to NATS requests to avoid sender timeouts
        if reply:
            if event_type == "call_initiated" and leg == "a_leg":
                caller = event_data.get("caller")
                callee = event_data.get("callee")
                log_info(f"Extension {caller} is dialing {callee}. Replying with 'dial' command...")
                
                # Send dial command to target callee via NATS Request-Reply
                instruction = {
                    "action": "dial",
                    "targets": [f"sip:{callee}@192.168.117.91:5091"],
                    "sim_ring": False,
                    "caller_id": caller,
                    "timeout_secs": 30,
                    "record_call": True
                }
                await nc.publish(reply, json.dumps(instruction).encode())
                log_success(f"Sent VCI Dial Instruction: {json.dumps(instruction)}")
            else:
                await nc.publish(reply, b"{}")

    # Subscribe to inbound call events
    sub = await nc.subscribe("vos_rs.call.incoming", cb=message_handler)
    log_info("Subscribed to 'vos_rs.call.incoming'. Dial extension (e.g. 10001 calling 15300002222) via SIP to trigger.")
    
    try:
        while True:
            await asyncio.sleep(1)
    except KeyboardInterrupt:
        await sub.unsubscribe()

async def run_double_sided_callback_scenario(nc, caller_ext, callee_num, host_ip):
    """
    Scenario 2: 双向回拨 (先呼分机 A-leg，再呼被叫 B-leg，最后桥接)
    """
    call_id_a = f"leg_a_{uuid.uuid4().hex[:8]}"
    call_id_b = f"leg_b_{uuid.uuid4().hex[:8]}"
    
    log_info(f"Starting Scenario 2: Double-Sided Callback (using host IP: {host_ip})...")
    log_info(f"Step 1: Originate call to Extension Leg A ({caller_ext}) with Call-ID: {call_id_a}")

    # Listen to events to track progress of A-leg and B-leg
    a_leg_answered = asyncio.Event()
    b_leg_answered = asyncio.Event()

    async def event_handler(msg):
        data = json.loads(msg.data.decode())
        event_type = data.get("event_type")
        cid = data.get("call_id")
        event_data = data.get("data", {})
        leg = event_data.get("leg")

        log_info(f"Event Notification: {colors.BOLD}{event_type}{colors.ENDC} | Call-ID: {cid} | Leg: {leg}")

        # Always reply to avoid sender timeouts
        if msg.reply:
            await nc.publish(msg.reply, b"{}")

        if cid == call_id_a and event_type == "call_answered":
            log_success("Leg A (Extension) answered the call!")
            a_leg_answered.set()
        elif cid == call_id_b and event_type == "call_answered":
            log_success("Leg B (Callee) answered the call!")
            b_leg_answered.set()

    sub = await nc.subscribe("vos_rs.call.incoming", cb=event_handler)

    # 1. Originate Leg A
    originate_a = {
        "call_id": call_id_a,
        "action": "originate",
        "target_uri": f"sip:{caller_ext}@192.168.117.90:5090",
        "caller_id": "Platform"
    }
    await nc.publish("vos_rs.call.commands", json.dumps(originate_a).encode())
    log_info(f"Sent Originate Leg A Command: {json.dumps(originate_a)}")

    # Wait for Leg A to answer
    try:
        await asyncio.wait_for(a_leg_answered.wait(), timeout=30)
    except asyncio.TimeoutError:
        log_error("Timeout waiting for Leg A to answer. Make sure extension is registered and answers call.")
        await sub.unsubscribe()
        return

    # 2. Originate Leg B
    log_info(f"Step 2: Leg A answered. Originating call to Callee Leg B ({callee_num}) with Call-ID: {call_id_b}")
    originate_b = {
        "call_id": call_id_b,
        "action": "originate",
        "target_uri": f"sip:{callee_num}@192.168.117.91:5091",
        "caller_id": caller_ext
    }
    await nc.publish("vos_rs.call.commands", json.dumps(originate_b).encode())
    log_info(f"Sent Originate Leg B Command: {json.dumps(originate_b)}")

    # Wait for Leg B to answer
    try:
        await asyncio.wait_for(b_leg_answered.wait(), timeout=30)
    except asyncio.TimeoutError:
        log_error("Timeout waiting for Leg B to answer.")
        await sub.unsubscribe()
        return

    # 3. Bridge Leg A and Leg B
    log_info("Step 3: Both legs answered. Pairing and bridging media...")
    bridge_cmd = {
        "call_id": f"bridge_{uuid.uuid4().hex[:8]}",
        "action": "bridge",
        "call_id_a": call_id_a,
        "call_id_b": call_id_b
    }
    await nc.publish("vos_rs.call.commands", json.dumps(bridge_cmd).encode())
    log_success(f"Sent Bridge Command: {json.dumps(bridge_cmd)}")
    log_success("Double-sided call successfully bridged!")

    await asyncio.sleep(5)
    await sub.unsubscribe()

async def run_conference_scenario(nc, caller_ext, callee_num, room_id, host_ip):
    """
    Scenario 3: 会议室 (Joining conference legs independently)
    """
    call_id_a = f"leg_a_{uuid.uuid4().hex[:8]}"
    call_id_b = f"leg_b_{uuid.uuid4().hex[:8]}"
    
    log_info(f"Starting Scenario 3: Conference Room ({room_id}) using host IP: {host_ip}...")
    
    a_leg_answered = asyncio.Event()
    b_leg_answered = asyncio.Event()

    async def event_handler(msg):
        data = json.loads(msg.data.decode())
        event_type = data.get("event_type")
        cid = data.get("call_id")
        event_data = data.get("data", {})
        leg = event_data.get("leg")

        log_info(f"Event Notification: {colors.BOLD}{event_type}{colors.ENDC} | Call-ID: {cid} | Leg: {leg}")

        # Always reply to avoid sender timeouts
        if msg.reply:
            await nc.publish(msg.reply, b"{}")

        if cid == call_id_a and event_type == "call_answered":
            log_success("Leg A (Extension) answered. Pushing to conference room...")
            a_leg_answered.set()
        elif cid == call_id_b and event_type == "call_answered":
            log_success("Leg B (Callee) answered. Pushing to conference room...")
            b_leg_answered.set()

    sub = await nc.subscribe("vos_rs.call.incoming", cb=event_handler)

    # 1. Originate Leg A and push to conference
    originate_a = {
        "call_id": call_id_a,
        "action": "originate",
        "target_uri": f"sip:{caller_ext}@192.168.117.90:5090",
        "caller_id": "Platform"
    }
    await nc.publish("vos_rs.call.commands", json.dumps(originate_a).encode())
    await a_leg_answered.wait()

    conf_a = {
        "call_id": call_id_a,
        "action": "conference",
        "room_id": room_id,
        "start_muted": False
    }
    await nc.publish("vos_rs.call.commands", json.dumps(conf_a).encode())
    log_success(f"Leg A successfully joined conference room {room_id}")

    # 2. Originate Leg B and push to conference
    originate_b = {
        "call_id": call_id_b,
        "action": "originate",
        "target_uri": f"sip:{callee_num}@192.168.117.91:5091",
        "caller_id": "Platform"
    }
    await nc.publish("vos_rs.call.commands", json.dumps(originate_b).encode())
    await b_leg_answered.wait()

    conf_b = {
        "call_id": call_id_b,
        "action": "conference",
        "room_id": room_id,
        "start_muted": False
    }
    await nc.publish("vos_rs.call.commands", json.dumps(conf_b).encode())
    log_success(f"Leg B successfully joined conference room {room_id}")
    log_success("Conference scenario completed!")
 
    await asyncio.sleep(5)
    await sub.unsubscribe()
 
async def run_reverse_callback_scenario(nc, caller_ext, callee_num, host_ip):
    """
    Scenario 4: 反向双向回拨 (先呼被叫 B-leg，再呼分机 A-leg，最后桥接)
    """
    call_id_b = f"leg_b_{uuid.uuid4().hex[:8]}"
    call_id_a = f"leg_a_{uuid.uuid4().hex[:8]}"
    
    log_info(f"Starting Scenario 4: Reverse Callback (using host IP: {host_ip})...")
    log_info(f"Step 1: Originate call to Callee Leg B ({callee_num}) with Call-ID: {call_id_b}")
 
    # Listen to events to track progress of A-leg and B-leg
    a_leg_answered = asyncio.Event()
    b_leg_answered = asyncio.Event()
 
    async def event_handler(msg):
        data = json.loads(msg.data.decode())
        event_type = data.get("event_type")
        cid = data.get("call_id")
        event_data = data.get("data", {})
        leg = event_data.get("leg")
 
        log_info(f"Event Notification: {colors.BOLD}{event_type}{colors.ENDC} | Call-ID: {cid} | Leg: {leg}")
 
        # Always reply to avoid sender timeouts
        if msg.reply:
            await nc.publish(msg.reply, b"{}")
 
        if cid == call_id_a and event_type == "call_answered":
            log_success("Leg A (Extension) answered the call!")
            a_leg_answered.set()
        elif cid == call_id_b and event_type == "call_answered":
            log_success("Leg B (Callee) answered the call!")
            b_leg_answered.set()
 
    sub = await nc.subscribe("vos_rs.call.incoming", cb=event_handler)
 
    # 1. Originate Leg B (Callee) first
    originate_b = {
        "call_id": call_id_b,
        "action": "originate",
        "target_uri": f"sip:{callee_num}@192.168.117.91:5091",
        "caller_id": "Platform"
    }
    await nc.publish("vos_rs.call.commands", json.dumps(originate_b).encode())
    log_info(f"Sent Originate Leg B Command: {json.dumps(originate_b)}")
 
    # Wait for Leg B to answer
    try:
        await asyncio.wait_for(b_leg_answered.wait(), timeout=30)
    except asyncio.TimeoutError:
        log_error("Timeout waiting for Leg B to answer. Make sure callee is registered and answers.")
        await sub.unsubscribe()
        return
 
    # 2. Originate Leg A (Extension)
    log_info(f"Step 2: Leg B answered. Originating call to Extension Leg A ({caller_ext}) with Call-ID: {call_id_a}")
    originate_a = {
        "call_id": call_id_a,
        "action": "originate",
        "target_uri": f"sip:{caller_ext}@192.168.117.90:5090",
        "caller_id": callee_num
    }
    await nc.publish("vos_rs.call.commands", json.dumps(originate_a).encode())
    log_info(f"Sent Originate Leg A Command: {json.dumps(originate_a)}")
 
    # Wait for Leg A to answer
    try:
        await asyncio.wait_for(a_leg_answered.wait(), timeout=30)
    except asyncio.TimeoutError:
        log_error("Timeout waiting for Leg A to answer.")
        await sub.unsubscribe()
        return
 
    # 3. Bridge Leg A and Leg B
    log_info("Step 3: Both legs answered. Pairing and bridging media...")
    bridge_cmd = {
        "call_id": f"bridge_{uuid.uuid4().hex[:8]}",
        "action": "bridge",
        "call_id_a": call_id_a,
        "call_id_b": call_id_b
    }
    await nc.publish("vos_rs.call.commands", json.dumps(bridge_cmd).encode())
    log_success(f"Sent Bridge Command: {json.dumps(bridge_cmd)}")
    log_success("Double-sided call successfully bridged (Reverse Callback)!")
 
    await asyncio.sleep(5)
    await sub.unsubscribe()
 
async def main():
    parser = argparse.ArgumentParser(description="VOS-RS VCI & Leg Scenarios Verification Script")
    parser.add_argument("--nats-url", default="nats://127.0.0.1:4222", help="NATS Server Address")
    parser.add_argument("--scenario", type=int, choices=[1, 2, 3, 4], required=True, 
                        help="1: Inbound Auto Dial, 2: Double-Sided Callback, 3: Conference Room, 4: Reverse Callback")
    parser.add_argument("--caller", default="10001", help="Leg A Caller Extension (default: 10001)")
    parser.add_argument("--callee", default="15300002222", help="Leg B Callee number (default: 15300002222)")
    parser.add_argument("--room", default="room_8888", help="Conference room identifier (for scenario 3)")
 
    args = parser.parse_args()
 
    nc = NATS()
    try:
        log_info(f"Connecting to NATS Server at {args.nats_url}...")
        await nc.connect(args.nats_url)
        log_success("Connected to NATS!")
    except Exception as e:
        log_error(f"Failed to connect to NATS: {e}")
        sys.exit(1)
 
    import socket
    try:
        host_ip = socket.gethostbyname("host.docker.internal")
    except Exception:
        host_ip = "0.250.250.254"
    log_info(f"Resolved host.docker.internal to {host_ip}")
 
    try:
        if args.scenario == 1:
            await run_inbound_dial_scenario(nc, args.nats_url, host_ip)
        elif args.scenario == 2:
            await run_double_sided_callback_scenario(nc, args.caller, args.callee, host_ip)
        elif args.scenario == 3:
            await run_conference_scenario(nc, args.caller, args.callee, args.room, host_ip)
        elif args.scenario == 4:
            await run_reverse_callback_scenario(nc, args.caller, args.callee, host_ip)
    finally:
        await nc.close()
        log_info("NATS Connection closed.")

if __name__ == "__main__":
    asyncio.run(main())
