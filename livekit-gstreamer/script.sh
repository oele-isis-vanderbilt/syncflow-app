#!/bin/bash

# gst-launch-1.0 -e \
#   alsasrc device=hw:1,0 buffer-time=200000 latency-time=10000 ! \
#   audio/x-raw,format=S32LE,rate=96000,channels=10,layout=interleaved ! \
#   deinterleave name=di \
#   di.src_0 ! queue ! audioconvert ! audioresample \
#            ! audio/x-raw,format=S16LE,rate=96000,channels=1 \
#            ! voaacenc bitrate=256000 ! aacparse ! mp4mux faststart=true ! filesink location=ch1.m4a \
#   di.src_1 ! queue ! audioconvert ! audioresample \
#            ! audio/x-raw,format=S16LE,rate=96000,channels=1 \
#            ! voaacenc bitrate=256000 ! aacparse ! mp4mux faststart=true ! filesink location=ch2.m4a \
#   di.src_2 ! queue ! audioconvert ! audioresample \
#            ! audio/x-raw,format=S16LE,rate=96000,channels=1 \
#            ! voaacenc bitrate=256000 ! aacparse ! mp4mux faststart=true ! filesink location=ch3.m4a \
#   di.src_3 ! queue ! audioconvert ! audioresample \
#            ! audio/x-raw,format=S16LE,rate=96000,channels=1 \
#            ! voaacenc bitrate=256000 ! aacparse ! mp4mux faststart=true ! filesink location=ch4.m4a \
#   di.src_4 ! queue ! audioconvert ! audioresample \
#            ! audio/x-raw,format=S16LE,rate=96000,channels=1 \
#            ! voaacenc bitrate=256000 ! aacparse ! mp4mux faststart=true ! filesink location=ch5.m4a \
#   di.src_5 ! queue ! audioconvert ! audioresample \
#            ! audio/x-raw,format=S16LE,rate=96000,channels=1 \
#            ! voaacenc bitrate=256000 ! aacparse ! mp4mux faststart=true ! filesink location=ch6.m4a \
#   di.src_6 ! queue ! audioconvert ! audioresample \
#            ! audio/x-raw,format=S16LE,rate=96000,channels=1 \
#            ! voaacenc bitrate=256000 ! aacparse ! mp4mux faststart=true ! filesink location=ch7.m4a \
#   di.src_7 ! queue ! audioconvert ! audioresample \
#            ! audio/x-raw,format=S16LE,rate=96000,channels=1 \
#            ! voaacenc bitrate=256000 ! aacparse ! mp4mux faststart=true ! filesink location=ch8.m4a

gst-launch-1.0 -e \
  alsasrc device=hw:1,0 buffer-time=200000 latency-time=10000 ! \
  audio/x-raw,format=S32LE,rate=96000,channels=10,layout=interleaved ! \
  deinterleave name=di \
  \
  di.src_0 ! tee name=t0 \
    t0. ! queue ! audioconvert ! audioresample ! audio/x-raw,format=S16LE,rate=96000,channels=1 \
         ! voaacenc bitrate=256000 ! aacparse ! mp4mux faststart=true ! filesink location=ch1.m4a \
    t0. ! queue ! audioconvert ! audioresample ! audioamplify amplification=0.125 ! audiomixer name=mixL \
  \
  di.src_1 ! tee name=t1 \
    t1. ! queue ! audioconvert ! audioresample ! audio/x-raw,format=S16LE,rate=96000,channels=1 \
         ! voaacenc bitrate=256000 ! aacparse ! mp4mux faststart=true ! filesink location=ch2.m4a \
    t1. ! queue ! audioconvert ! audioresample ! audioamplify amplification=0.125 ! audiomixer name=mixR \
  \
  di.src_2 ! tee name=t2 \
    t2. ! queue ! audioconvert ! audioresample ! audio/x-raw,format=S16LE,rate=96000,channels=1 \
         ! voaacenc bitrate=256000 ! aacparse ! mp4mux faststart=true ! filesink location=ch3.m4a \
    t2. ! queue ! audioconvert ! audioresample ! audioamplify amplification=0.125 ! mixL. \
  \
  di.src_3 ! tee name=t3 \
    t3. ! queue ! audioconvert ! audioresample ! audio/x-raw,format=S16LE,rate=96000,channels=1 \
         ! voaacenc bitrate=256000 ! aacparse ! mp4mux faststart=true ! filesink location=ch4.m4a \
    t3. ! queue ! audioconvert ! audioresample ! audioamplify amplification=0.125 ! mixR. \
  \
  di.src_4 ! tee name=t4 \
    t4. ! queue ! audioconvert ! audioresample ! audio/x-raw,format=S16LE,rate=96000,channels=1 \
         ! voaacenc bitrate=256000 ! aacparse ! mp4mux faststart=true ! filesink location=ch5.m4a \
    t4. ! queue ! audioconvert ! audioresample ! audioamplify amplification=0.125 ! mixL. \
  \
  di.src_5 ! tee name=t5 \
    t5. ! queue ! audioconvert ! audioresample ! audio/x-raw,format=S16LE,rate=96000,channels=1 \
         ! voaacenc bitrate=256000 ! aacparse ! mp4mux faststart=true ! filesink location=ch6.m4a \
    t5. ! queue ! audioconvert ! audioresample ! audioamplify amplification=0.125 ! mixR. \
  \
  di.src_6 ! tee name=t6 \
    t6. ! queue ! audioconvert ! audioresample ! audio/x-raw,format=S16LE,rate=96000,channels=1 \
         ! voaacenc bitrate=256000 ! aacparse ! mp4mux faststart=true ! filesink location=ch7.m4a \
    t6. ! queue ! audioconvert ! audioresample ! audioamplify amplification=0.125 ! mixL. \
  \
  di.src_7 ! tee name=t7 \
    t7. ! queue ! audioconvert ! audioresample ! audio/x-raw,format=S16LE,rate=96000,channels=1 \
         ! voaacenc bitrate=256000 ! aacparse ! mp4mux faststart=true ! filesink location=ch8.m4a \
    t7. ! queue ! audioconvert ! audioresample ! audioamplify amplification=0.125 ! mixR. \
  \
  mixL. ! queue ! audioconvert ! audioresample ! audio/x-raw,channels=1,rate=96000,format=S16LE ! interleave name=il \
  mixR. ! queue ! audioconvert ! audioresample ! audio/x-raw,channels=1,rate=96000,format=S16LE ! il. \
  il.   ! queue ! audio/x-raw,channels=2,rate=96000,format=S16LE ! autoaudiosink
